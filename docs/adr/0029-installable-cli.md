# 0029 — An installable CLI, authenticated by tailnet identity

**Status:** accepted · **Date:** 2026-08-30 · relates to [0016](0016-in-dashboard-node-terminal.md) (terminal + `/tsauth` identity), [0014](0014-managed-db-access-via-adminer.md) (managed DB access), [0008](0008-ci-built-control-plane-images.md) (no toolchain on nodes), design §12.1 (WG bind trust), §15 (managed databases), §16 (imperative escape hatches)

## Context

The read-only `majnet` CLI (phase 0) made the fleet diagnosable without a
browser, but only from a WireGuard peer, and only for reading. Everything else
still meant opening the dashboard: promoting a build, cutting a release,
restarting a container, reading logs during an incident. Two consequences:

- **Nothing was scriptable.** Not a smoke test after a deploy, not a runbook
  step, not an agent asked to look into a failing app. The dashboard is a
  browser app; its API is reachable only through a front door that injects an
  identity header.
- **Two capabilities had no non-interactive form at all.** Running a command
  inside an app container existed only as the ADR 0016 terminal — a PTY over a
  WebSocket, platform-admin only, which cannot be scripted and gives a *host*
  root shell across the whole fleet. Querying an app's database existed only as
  Adminer in a browser (ADR 0014), scoped per project, with no way to run one
  statement and read the answer.

The obvious way to close that gap is an API token. That would have been wrong
here: the platform's whole access story is the tailnet. `tailscale serve` (and
the Caddy edge via the bot's `/tsauth`) already resolves a caller's tailnet IP
to a login and injects `Tailscale-User-Login`, which `people.yaml` and each
project's `project.yaml` turn into roles. Issuing tokens would have created a
second, weaker credential path — one to mint, store, rotate and revoke — beside
an identity the platform already trusts.

## Decision

**One installable `majnet` binary, authenticated by the same tailnet identity
the dashboard uses, covering the whole API.**

1. **No credential of its own.** The CLI is an HTTP client pointed at the
   dashboard origin. Identity is injected at that front door from the calling
   device's tailnet IP; roles come from `people.yaml` + `project.yaml`, edited
   in the UI. Permissions granted in the dashboard *are* the CLI's permissions.
   `~/.config/majnet/config.yaml` holds URLs and defaults, nothing secret.

2. **Three identities, always named.** A request without a resolved identity is
   not refused — the backends read it as `infra`, the WG-mesh break-glass
   (§12.1), and pass every role check; `/api/whoami` then answers
   `{login: null, admin: true}`. Separately, an unauthenticated `/api` request
   to the dashboard falls through to the SPA and returns **200 `text/html`**.
   Both look like success. `majnet whoami` reports which of *human / infra /
   not-the-API* you are, and the HTTP layer turns the SPA fallthrough into a
   loud error rather than an empty list.

3. **Two new reconciler endpoints**, because the interactive equivalents cannot
   be scripted:
   - `POST /api/exec/{project}/{class}/{app}` — one command in the app's
     container, returning stdout, stderr and the exit code.
   - `POST /api/sql/{project}/{class}/{app}` (plus `GET /api/db/…`) — one
     statement against the app's managed database.

   Both are gated **exactly like `/api/logs`**: production needs a project
   admin, other classes a developer. Both write an audit event naming the caller
   and what was run.

4. **SQL runs as the app's own role.** The reconciler execs the engine's client
   inside the engine container using the per-`(project, app, class)` credential
   derived in `db.rs` (§15) — never the superuser. Without `write=true` the
   statement runs in a read-only transaction, and a write requires project admin
   in *every* class, not just production.

5. **Distribution follows ADR 0008's rule that nodes hold no toolchain.**
   Cross-compiled binaries are attached to a GitHub release
   (`.github/workflows/cli-release.yaml`) and installed by
   `scripts/install-cli.sh`; the control-plane image keeps shipping the same
   binary for node-local use, pre-pointed at the WG listeners.

6. **The CLI documents itself for machines.** `majnet agent-guide` prints an
   agent-facing reference compiled into the binary, and `--install` writes it
   into a repo as a Claude Code skill. `--output json` returns the API's JSON
   unmodified, and is the default when stdout is not a terminal.

## Consequences

**The role model finally reaches a terminal.** A project developer can read
logs, restart, exec and query their non-production databases without a browser
and without anyone minting them anything. Onboarding is `tailscale up` plus a
line in `people.yaml`; offboarding is removing the tailnet device.

**`exec` widens what a project developer can do.** Until now no non-platform-
admin could run a command in a container. That is a deliberate widening, and it
is bounded: one command, in one container, of an app they already administer —
the same blast radius as `restart` and the manifest editor, and strictly less
than the ADR 0016 terminal, which stays platform-admin-only because it hands out
a host root shell. Every call is audited.

**The read-only SQL guard is a seatbelt, not a sandbox.** A read-only
transaction stops a mistyped `UPDATE`; it does not stop someone who opens
another transaction. This is stated in the code, the docs and the agent guide
rather than implied away, because the mitigations that *are* real — the app-role
credential, the admin gate on writes, the audit row — are weakened by pretending
there is a boundary that isn't there.

**Identity now has to be right, loudly.** Because `infra` passes every check,
the CLI's most important behaviour is refusing to render an unresolved identity
as a name, and refusing to read an HTML 200 as an empty result. Both are
unit-tested; the second is the failure that already made a probe look successful
during an incident.

**`--direct` remains, narrowly.** Talking to the WG listeners from a node keeps
the phase-0 break-glass working (and is how the in-image CLI is configured), but
it sends no identity, so `majnet shell` refuses it outright — a recorded
terminal session must be attributable to a named admin.

## Alternatives considered

**API tokens.** A second credential to mint, store, rotate and revoke, weaker
than the tailnet identity it would sit beside, and a new way to lose access to
the platform. Rejected.

**A shell wrapper over `curl`.** Cheapest, and it would have inherited the exact
failure this CLI exists to prevent: the SPA fallthrough returns 200 with HTML,
which a shell pipeline reads as an empty result.

**Exposing SQL through the terminal WebSocket.** Would have made every query
platform-admin-only and un-scriptable, and left project members where they
started — in a browser, in Adminer.
