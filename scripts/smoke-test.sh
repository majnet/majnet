#!/usr/bin/env bash
# End-to-end smoke test of the reconciler against the LOCAL Docker daemon.
#
# Exercises the §12 loop without GitHub or any server:
#   fixture env branch → converge → healthy container with a decrypted inline
#   secret on tmpfs → blue-green on config change → exec in the container →
#   SQL against a provisioned postgres → GC when config is gone.
#
# The exec/sql steps (ADR 0029) run twice over: first with no identity header,
# where the reconciler sees `infra` (the WG bind is the credential, §12.1) and
# every role check passes — that covers the Docker and engine mechanics; then
# with a `Tailscale-User-Login` header for each of the three tiers in the
# fixture's people.yaml, which covers the gate itself.
#
# Requires: docker, age, age-keygen, cargo (all in the nix dev shell).
# Usage: scripts/smoke-test.sh   (from the repo root; direnv or nix develop)

set -euo pipefail
cd "$(dirname "$0")/.."

APP=hello
# A second app, added later, exists only to carry `database: postgres` — so the
# steps above it stay fast and independent of a postgres pull.
DBAPP=hellodb
PROJECT=demo
IMAGE=nginx:1.27-alpine   # has busybox wget for the health check
# Same helper the reconciler uses to place files on a host (platform.rs).
HELPER_IMAGE=busybox:stable
LISTEN=127.0.0.1:19090
WORK=$(mktemp -d)
RECON_PID=

red()   { printf '\033[31m✗ %s\033[0m\n' "$*"; }
green() { printf '\033[32m✓ %s\033[0m\n' "$*"; }
step()  { printf '\033[1;34m── %s\033[0m\n' "$*"; }

cleanup() {
  if [[ -n $RECON_PID ]]; then kill "$RECON_PID" 2>/dev/null || true; fi
  docker ps -aq --filter "label=majnet.project=$PROJECT" | xargs -r docker rm -f >/dev/null 2>&1 || true
  docker network rm "proj-$PROJECT" >/dev/null 2>&1 || true
  # The managed engine and its data volume are platform-scoped, so they carry no
  # project label and have to be named explicitly.
  docker rm -f majnet-postgres >/dev/null 2>&1 || true
  docker volume rm -f postgres-data >/dev/null 2>&1 || true
  docker ps -aq --filter "label=majnet.helper=platform" | xargs -r docker rm -f >/dev/null 2>&1 || true
  # The engine's root-secret file is written *inside* a container, so it lands
  # owned by root and a plain `rm -rf` cannot remove it — which would leave the
  # trap returning non-zero and turn a passing run into a failing one. Delete it
  # the same way it was created.
  if [[ -d $WORK/db-root ]]; then
    docker run --rm -v "$WORK/db-root:/d" "$HELPER_IMAGE" \
      sh -c 'rm -rf /d/..?* /d/.[!.]* /d/*' >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK" 2>/dev/null || true
}
trap cleanup EXIT

fail() { red "$1"; echo "--- reconciler log tail ---"; tail -30 "$WORK/reconciler.log" 2>/dev/null || true; exit 1; }

# wait_for <seconds> <description> <command...>
wait_for() {
  local timeout=$1 what=$2; shift 2
  for _ in $(seq 1 "$timeout"); do
    if "$@" >/dev/null 2>&1; then green "$what"; return 0; fi
    sleep 1
  done
  fail "timed out waiting for: $what"
}

app_container() {
  docker ps -q --filter "label=majnet.project=$PROJECT" --filter "label=majnet.app=$APP" --filter status=running
}
app_is_healthy() {
  local id; id=$(app_container); [[ -n $id ]] || return 1
  [[ $(docker inspect -f '{{.State.Health.Status}}' "$id") == healthy ]]
}
app_env_rev() {
  local id; id=$(app_container); [[ -n $id ]] || return 1
  docker inspect -f '{{range .Config.Env}}{{println .}}{{end}}' "$id" | grep -q "^REV=$1$"
}
app_gone() { [[ -z $(app_container) ]]; }
app_single() { [[ $(app_container | wc -l | tr -d ' ') == 1 ]]; }
db_app_healthy() {
  local id
  id=$(docker ps -q --filter "label=majnet.project=$PROJECT" --filter "label=majnet.app=$DBAPP" --filter status=running)
  [[ -n $id ]]
}
pg_ready() { docker exec majnet-postgres pg_isready -U postgres -q; }

notify() { curl -fs -X POST "http://$LISTEN/notify" -H 'content-type: application/json' -d '{}' >/dev/null; }

# api_post <path> <json-body> — fails the test on any non-2xx.
api_post() {
  curl -fsS -X POST "http://$LISTEN$1" -H 'content-type: application/json' -d "$2"
}
# api_status <path> <json-body> — prints the HTTP status, body into $WORK/body.
# For the cases where a refusal IS the expected outcome.
api_status() {
  curl -sS -o "$WORK/body" -w '%{http_code}' \
    -X POST "http://$LISTEN$1" -H 'content-type: application/json' -d "$2"
}
# api_status_as <tailscale-login> <path> <json-body> — the same, as a named
# human. The header is what the dashboard's front door injects from the caller's
# tailnet IP; here we set it directly, which is exactly what the trust model
# says must never be reachable from outside that front door.
api_status_as() {
  curl -sS -o "$WORK/body" -w '%{http_code}' \
    -X POST "http://$LISTEN$2" \
    -H 'content-type: application/json' \
    -H "Tailscale-User-Login: $1" \
    -d "$3"
}

# has <needle> <haystack> — literal match, so JSON escapes need no quoting care.
has() { grep -qF "$1" <<<"$2"; }

# denied <what> <status> — a role check must refuse with 403, not blow up with a
# 502 somewhere deeper. Getting that wrong would mean the request reached Docker.
denied() {
  [[ $2 == 403 ]] || fail "$1: expected 403, got $2 — $(cat "$WORK/body")"
  green "$1"
}
allowed() {
  [[ $2 == 2* ]] || fail "$1: expected success, got $2 — $(cat "$WORK/body")"
  green "$1"
}

step "preflight"
docker info >/dev/null || { red "docker daemon not reachable"; exit 1; }
{ command -v age && command -v age-keygen; } >/dev/null || { red "need age + age-keygen (nix dev shell)"; exit 1; }

step "building fixture in $WORK"
SNAP="$WORK/snapshots"
# The project's env-branch dirs are created here (git tracks no empty dirs).
mkdir -p "$SNAP" "$WORK/age" "$SNAP/$PROJECT/ops/env/stable"
cp -R scripts/smoke/fixture/* "$SNAP/"

# Class age key + one inline-encrypted secret (ADR 0024): the manifest carries a
# `majnet:<base64(age ciphertext)>` envelope the reconciler decrypts with the
# class key — exactly what the bot's encrypt endpoint embeds. SOPS was retired.
age-keygen -o "$WORK/age/age-stable.key" 2>/dev/null
# The DB master key (§15): every per-app password is HMAC(master, …), so the
# reconciler stores no credentials. A fixed value keeps the test deterministic —
# it protects nothing here, and the real one is generated at install.
printf 'smoke-test-db-master-key-not-a-secret' > "$WORK/age/db-master.key"
AGE_PUB=$(age-keygen -y "$WORK/age/age-stable.key")
SECRET_ENVELOPE="majnet:$(printf 'hello-from-sops' | age -r "$AGE_PUB" | base64 | tr -d '\n')"

# Digest-pinned image, like a rendered manifest.
docker pull -q "$IMAGE" >/dev/null
DIGEST=$(docker inspect -f '{{index .RepoDigests 0}}' "$IMAGE")
manifest() { # manifest <rev>
  cat > "$SNAP/$PROJECT/ops/env/stable/$APP.yaml" <<EOF
name: $APP
image: $DIGEST
env:
  REV: "$1"
secrets:
  greeting: $SECRET_ENVELOPE
health:
  path: /
  port: 80
EOF
}
# Same image, plus a managed database. Written only when the SQL step runs, so
# nothing before it waits on a postgres pull.
db_manifest() {
  cat > "$SNAP/$PROJECT/ops/env/stable/$DBAPP.yaml" <<EOF
name: $DBAPP
image: $DIGEST
database:
  engine: postgres
health:
  path: /
  port: 80
EOF
}
manifest 1
green "fixture ready ($DIGEST)"

step "building + starting reconciler (local mode)"
cargo build -q -p majnet-reconciler
MAJNET_BOT_URL=http://unused.invalid \
MAJNET_DOCKER_LOCAL=1 \
MAJNET_SNAPSHOT_DIR="$SNAP" \
MAJNET_AGE_KEY_DIR="$WORK/age" \
MAJNET_DB_ROOT_DIR="$WORK/db-root" \
MAJNET_DATA_DIR="$WORK/data" \
MAJNET_LISTEN="$LISTEN" \
MAJNET_POLL_INTERVAL_SECS=3 \
RUST_LOG=info \
  target/debug/majnet-reconciler > "$WORK/reconciler.log" 2>&1 &
RECON_PID=$!
wait_for 10 "reconciler is up" curl -fs "http://$LISTEN/healthz"

step "1) initial converge: container healthy, secret decrypted onto tmpfs"
wait_for 90 "app container healthy" app_is_healthy
if docker exec "$(app_container)" sh -c 'test "$(cat /run/secrets/greeting)" = hello-from-sops'; then
  green "inline secret decrypted and mounted at /run/secrets/greeting"
else
  fail "secret file wrong or missing"
fi
docker network inspect "proj-$PROJECT" >/dev/null 2>&1 && green "project network exists"

step "2) blue-green: config change replaces the container, no gap"
OLD_ID=$(app_container)
manifest 2
notify
wait_for 90 "new container serving REV=2" app_env_rev 2
if [[ $(app_container) != "$OLD_ID" ]]; then green "old container replaced"; else fail "container was not replaced"; fi
wait_for 30 "exactly one container remains" app_single
app_is_healthy && green "replacement is healthy"

step "3) exec: one command inside the app container (ADR 0029)"
EXEC="/api/exec/$PROJECT/stable/$APP"

# Proves it reached the *right* container: the decrypted secret exists nowhere else.
out=$(api_post "$EXEC" '{"cmd":["cat","/run/secrets/greeting"]}')
has '"stdout":"hello-from-sops' "$out" || fail "exec ran somewhere else, or stdout is wrong: $out"
has '"exit_code":0' "$out" || fail "no exit_code in the response: $out"
green "exec ran in the app container"

# stdout and stderr stay apart — the reason the exec runs without a TTY, and
# what lets a caller parse one of them.
out=$(api_post "$EXEC" '{"cmd":["sh","-c","echo out; echo err >&2"]}')
if has '"stdout":"out\n"' "$out" && has '"stderr":"err\n"' "$out"; then
  green "stdout and stderr are separable"
else
  fail "streams merged or mangled: $out"
fi

out=$(api_post "$EXEC" '{"cmd":["cat"],"stdin":"ping"}')
has '"stdout":"ping"' "$out" || fail "stdin was not delivered: $out"
green "stdin reaches the command"

# A command that fails is a SUCCESSFUL request carrying a non-zero code. The
# CLI's exit status depends on that distinction, so assert it explicitly.
out=$(api_post "$EXEC" '{"cmd":["sh","-c","exit 7"]}')
has '"exit_code":7' "$out" || fail "exit code not propagated: $out"
green "a failing command is reported, not raised"

step "4) sql: a real statement against a provisioned postgres (ADR 0029)"
db_manifest
notify
# First converge of an app with a database also deploys the engine and waits for
# it, so this covers an image pull on a cold runner.
wait_for 300 "postgres engine ready" pg_ready
wait_for 120 "$DBAPP container running" db_app_healthy
SQL="/api/sql/$PROJECT/stable/$DBAPP"

out=$(api_post "$SQL" '{"sql":"SELECT 1 AS one"}')
has '"columns":["one"]' "$out" || fail "columns not parsed out of psql CSV: $out"
has '"rows":[["1"]]' "$out" || fail "rows not parsed out of psql CSV: $out"
has '"read_only":true' "$out" || fail "the response does not admit it was read-only: $out"
green "a read query returns parsed rows"

# The seatbelt: without write=true the statement runs in a read-only
# transaction, and postgres — not us — refuses it.
code=$(api_status "$SQL" '{"sql":"CREATE TABLE smoke(i int)"}')
[[ $code != 2* ]] || fail "a write was accepted without write=true (HTTP $code)"
grep -qi 'read-only' "$WORK/body" || fail "refused, but not for the read-only reason: $(cat "$WORK/body")"
green "a write is refused without write=true, by the engine"

api_post "$SQL?write=true" '{"sql":"CREATE TABLE smoke(i int)"}' >/dev/null \
  || fail "write=true did not allow the write"
green "write=true allows it"

out=$(api_post "$SQL?meta=tables" '{"sql":""}')
has 'smoke' "$out" || fail "meta=tables did not list the new table: $out"
green "meta=tables reads the catalog"

# The app and the query resolve to the same database, so the stateless
# credential derivation (§15) agrees end to end.
out=$(api_post "/api/exec/$PROJECT/stable/$DBAPP" '{"cmd":["printenv","DATABASE_URL"]}')
# Never echo this response — it carries the derived password.
has '"stdout":"postgres://demo_hellodb_stable:' "$out" \
  || fail "DATABASE_URL missing, or not pointing at the app's own database"
green "the app got a DATABASE_URL for the same database"

step "5) authorization: the identity header decides what you may do"
DEV=dev@smoke.invalid
PADMIN=admin@smoke.invalid
PLATFORM=platform@smoke.invalid
READ='{"cmd":["true"]}'

# A developer may act on a non-production class.
allowed "developer may exec on stable" \
  "$(api_status_as "$DEV" "/api/exec/$PROJECT/stable/$APP" "$READ")"

# …and may not on production. The gate runs before anything touches Docker, so
# this is a 403 even though no production container exists — which is the point:
# the refusal must not depend on what happens to be deployed.
denied "developer may not exec on production" \
  "$(api_status_as "$DEV" "/api/exec/$PROJECT/production/$APP" "$READ")"

# Reading a database is a developer's business; writing to one is not, in any
# class — the role model has no third tier, so writes take the higher one.
allowed "developer may read the database" \
  "$(api_status_as "$DEV" "/api/sql/$PROJECT/stable/$DBAPP" '{"sql":"SELECT 1"}')"
denied "developer may not write to it" \
  "$(api_status_as "$DEV" "/api/sql/$PROJECT/stable/$DBAPP?write=true" '{"sql":"SELECT 1"}')"

# A project admin may.
allowed "project admin may write" \
  "$(api_status_as "$PADMIN" "/api/sql/$PROJECT/stable/$DBAPP?write=true" \
     '{"sql":"CREATE TABLE IF NOT EXISTS smoke_authz(i int)"}')"

# A platform admin passes every project gate. There is no production container,
# so this gets as far as Docker and fails there (502) — which is itself the
# assertion: it was not refused.
code=$(api_status_as "$PLATFORM" "/api/exec/$PROJECT/production/$APP" "$READ")
[[ $code != 403 ]] || fail "platform admin was refused a project gate"
green "platform admin passes the project gate"

# An identity the platform does not know is refused outright, rather than
# falling through to the header-less `infra` path.
denied "an unknown tailnet login is refused" \
  "$(api_status_as "nobody@smoke.invalid" "/api/exec/$PROJECT/stable/$APP" "$READ")"

step "6) GC: manifests removed from git → containers removed"
rm "$SNAP/$PROJECT/ops/env/stable/$APP.yaml" "$SNAP/$PROJECT/ops/env/stable/$DBAPP.yaml"
notify
wait_for 60 "app container gone" app_gone

step "7) event log tells the story"
events=$(curl -fs "http://$LISTEN/api/events?limit=100")
has '"action":"converge hello"' "$events" && green "converge events recorded"
has '"action":"gc"' "$events" && green "gc event recorded"
# The audit trail is what makes exec and sql defensible at all — if it is not
# written, the endpoints should not ship.
has '"action":"exec hello (stable)"' "$events" || fail "exec was not audited"
has '"action":"sql hellodb (stable)"' "$events" || fail "sql was not audited"
green "exec and sql are audited"
# And attributed to the person, not to `infra` — an audit trail that cannot name
# who acted is not one.
has 'by smoke-dev' "$events" || fail "an identified call was audited as someone else"
has 'by smoke-admin' "$events" || fail "the admin write was not attributed"
green "identified calls are audited under the caller's name"

echo
green "SMOKE TEST PASSED — converge→blue-green→exec→sql→roles→GC all work against real Docker"
