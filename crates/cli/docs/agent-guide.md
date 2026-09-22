# Operating MajNet from the command line (`majnet`)

MajNet is a self-hosted GitOps PaaS. `majnet` is its CLI: one binary that talks
to the control plane over a Tailscale network and covers everything the web
dashboard does — fleet status, logs, deploys, releases, a shell inside a running
container, and SQL against an app's managed database.

This document is written for an agent driving the CLI on someone's behalf. It
says what each command does, what it costs if it is wrong, and which answers
look like success without being success.

---

## 1. The five things to know before running anything

1. **There is no token.** Identity is the Tailscale device. The control plane
   sits behind a front door that resolves the caller's tailnet IP to a login and
   injects it as a header the backends trust. Permissions come from
   `people.yaml` (who is a platform admin) and each project's `project.yaml`
   (who is an admin or developer on that project), both edited in the dashboard.
   Nothing is issued, copied, or revoked for the CLI.

2. **Check who you are first.** `majnet whoami`. If it says *unidentified*, the
   calls will be attributed to `infra` and will bypass project roles. Stop and
   report that rather than proceeding — see §7.

3. **Environment classes are the unit of risk.** Every app runs in up to four:
   `production`, `stable`, `testing`, `ephemeral`. The CLI defaults to `stable`.
   `production` requires a project **admin** for reads *and* writes, and every
   production-touching command prompts unless `--yes` is passed.

4. **Writes go through git.** `promote`, `rollback`, manifest changes and
   release cuts are commits and pull requests, not imperative deploys. Their
   output describes what was *written*; the deploy follows when a render PR
   merges. The exceptions — `restart`, `exec`, `sql` — change nothing git owns,
   and are individually role-gated and audited.

5. **Everything is recorded.** `exec`, `sql`, `restart` and `shell` write an
   audit event naming the caller and what was run. `shell` additionally records
   the full session transcript. Do not run anything you would not want read back.

---

## 2. Setup

```sh
majnet login                       # find the control plane on the tailnet and verify identity
majnet login --url http://majksa   # or name it directly
majnet whoami                      # who the platform thinks you are, and your role per project
```

Optional defaults so commands get shorter:

```sh
majnet context set --project demo --app api --class stable
majnet context list
```

Config lives at `~/.config/majnet/config.yaml` and contains **no credentials** —
just URLs and defaults. `MAJNET_URL` overrides it for one invocation, which is
the easiest way to run in CI or inside a container on a node.

---

## 3. Output contract

- `--output json` prints the control plane's JSON **unmodified**. Use this for
  anything you intend to parse.
- When stdout is not a terminal, JSON is already the default — a piped `majnet`
  emits JSON without being asked.
- `--output table` is for humans and elides long cells. Never parse it.
- Errors go to stderr and the process exits non-zero.
- `majnet exec` is special: it exits with the **container's** exit code, and
  passes the container's stdout/stderr through untouched.

---

## 4. Command reference

Every command takes `[project] [app]` positionally where relevant; both fall
back to the context defaults. `project` may be the project's name or its GitHub
org — either resolves.

### Read — safe, no side effects

| Command | What it answers |
|---|---|
| `majnet status` | One screen: identity, node health (incl. disk), deploys in flight, recent failures. **Start here.** |
| `majnet events [--failed] [--project P] [--limit N] [--follow]` | The activity feed. `--failed` filters to things that broke. `--bot` reads the git-side log instead. |
| `majnet nodes` | Registered nodes and their addresses. |
| `majnet metrics [--node N]` | Live CPU / memory / **disk** / container counts. With one node, also per-container detail. |
| `majnet projects` | The project registry. |
| `majnet apps [project]` | Apps in a project: declared classes, image, database, host. |
| `majnet app [project] [app]` | One app in full: image, classes, build info per environment, containers per environment. |
| `majnet ps [project] [app] -c CLASS` | Containers backing one app in one environment. |
| `majnet logs [project] [app] -c CLASS [-n 300] [--follow]` | Container logs. `--follow` polls. |
| `majnet info [project] [app]` | What each environment reported at its `/info` endpoint (build metadata). |
| `majnet manifest [project] [app]` | The manifest YAML as committed on ops `main`. |
| `majnet members [project]` | Project members and roles. |
| `majnet secrets [project] [app] -c CLASS [--reveal]` | Secret **names**; values only with `--reveal`. |
| `majnet db [project] [app] -c CLASS` | Which engine and database an app uses, and where it lives. |
| `majnet deploy list [project]` | Render PRs waiting to be merged. |
| `majnet deploy progress` | Rollouts in flight per app and environment. |
| `majnet release list [project] [app]` | Versions already cut. |
| `majnet release drafts` | Every app across the fleet with a release waiting to be cut. |
| `majnet release draft show [project] [app]` | The pending version and its generated changelog. |
| `majnet control-plane status` | Pinned vs published vs running control-plane version. |
| `majnet version` | The platform version pinned in the platform repo. |

### Write — has consequences

| Command | What it does | Who may |
|---|---|---|
| `majnet deploy restart [project] [app] -c CLASS` | Restarts containers. Imperative; nothing in git changes. | developer (admin for production) |
| `majnet deploy promote [project] [app]` | Copies the stable digest into the production overlay. A **gated render PR follows** — this does not deploy on its own. | project admin |
| `majnet deploy merge [project] N` | Merges a render PR. For `env/production` this **is** the deploy. | project admin |
| `majnet deploy close [project] N` | Closes a render PR without merging. | project admin |
| `majnet deploy rollback [project]` | Reverts the ops repo's `main` head; render PRs propagate it. | project admin |
| `majnet release cut [project] [app] --bump auto\|patch\|minor\|major` | Tags a new version and starts the build. `auto` derives the bump from conventional-commit messages. | project admin |
| `majnet release draft submit [project] [app]` | Cuts the pending draft. | project admin |
| `majnet release draft notes [project] [app] "…"` | Replaces the changelog text (`-` reads stdin). | project admin |
| `majnet release draft refresh\|discard [project] [app]` | Recompute / throw away the draft. | project admin |
| `majnet release promote [project] [app] VERSION` | Moves an already-cut version into the stable track. | project admin |
| `majnet exec … -- CMD` | Runs one command in the app's container. | developer (admin for production) |
| `majnet sql … [--write]` | Runs SQL against the app's database. | developer to read, **admin to write** |
| `majnet shell …` | Interactive shell (container or node host). | **platform admin, named** |
| `majnet control-plane pin --ref …` | Publishes a new control-plane pin. | platform admin |

---

## 5. Running commands inside an app

```sh
majnet exec demo api -c stable -- ls -la /app
majnet exec demo api -c stable --shell 'ps aux | head'
majnet exec demo api -c stable --stdin - -- sh -c 'cat > /tmp/x'
majnet exec demo api -c production --yes -- printenv APP_VERSION
```

- Argv is executed directly — no shell — unless `--shell` is passed.
- `--workdir` maps to the container exec option. There is no `--user`: the
  command runs as the image's user, deliberately — picking one would let a
  developer run as root in the app container, past what the app itself runs as.
- Output is capped at 1 MiB; the CLI says so on stderr when it truncates.
- One command has 120 seconds.
- **Exit code is the container's.** `majnet exec … -- test -f /app/x` is safe in
  an `if`.

For an interactive session use `majnet shell` — but note it is platform-admin
only, records a transcript, and needs a real terminal. For anything scripted,
`exec` is the right tool.

---

## 6. SQL against a managed database

```sh
majnet db demo api -c stable                       # which engine, which database
majnet sql demo api -c stable --tables             # list tables
majnet sql demo api -c stable --columns users      # describe one
majnet sql demo api -c stable 'SELECT count(*) FROM users'
majnet sql demo api -c stable --file report.sql --limit 1000
majnet sql demo api -c stable                      # a small REPL, on a terminal
echo 'SELECT 1' | majnet sql demo api -c stable    # or from a pipe
```

How it works: the reconciler runs the engine's own client inside the engine
container, authenticating as **the app's own database role** — never as the
superuser. A query has exactly the privileges the app has.

**Read-only is the default.** Without `--write`, the statement runs in a
read-only transaction. Treat that as a seatbelt against a mistyped `UPDATE`,
**not** as a sandbox: anyone able to run SQL at all can open another
transaction. The real boundary is the role check (writes need project admin in
every class) and the audit trail.

Engines: `postgres` and `mariadb` return `columns` + `rows`. `valkey` and
`mongodb` accept their own client's commands and return a `raw` string — check
which you have with `majnet db` before assuming a shape.

`--limit` caps what is *printed*; the statement still runs in full. When rows
were dropped, `truncated` is `true` in the JSON.

JSON shape:

```json
{
  "engine": "postgres",
  "database": "demo_api_stable",
  "read_only": true,
  "columns": ["id", "email"],
  "rows": [["1", "a@example.com"]],
  "row_count": 1,
  "truncated": false,
  "raw": null,
  "notice": null
}
```

Every value is a **string**; the engine's text output is not re-typed, because
guessing a JSON type for a `numeric` or a `timestamptz` would silently change
it.

---

## 7. Answers that look like success but are not

This is the section to re-read when something seems wrong.

**"Unidentified — the control plane sees this call as `infra`."**
The URL answered, but no Tailscale identity reached the API. The backends do not
reject an identity-less call; they treat it as the WireGuard-mesh break-glass
and *pass every role check*. `majnet whoami` reports `admin: true` in that state,
which means "nobody checked", not "you are an admin". Causes: this machine is
not on the tailnet; the URL is not the identity-injecting front door; or the
Tailscale login has no entry in `people.yaml`. **Do not proceed with writes** —
they would be recorded as `infra` rather than as a person.

**"…returned text/html, which is the dashboard's SPA shell, not the API."**
An `/api` request whose identity cannot be resolved falls through to the web app
and returns HTTP **200** with HTML. That is an authentication failure wearing a
success code. The CLI turns it into an error; a hand-rolled `curl` would show it
as an empty result. Never work around this by parsing the HTML.

**`converged: null` on `control-plane status`.**
The running build did not report its version. That is *unknown*, not *not
converged* — do not report a stuck rollout on the strength of it.

**`mergeable: null` on a render PR.**
GitHub is still computing it. Wait and re-read; it is not a refusal.

**An empty `majnet ps` with a healthy `majnet apps`.**
The app is declared for that class but nothing is running there — often the
class was never rendered, or a deploy failed. `majnet events --failed --project
<p>` usually names the reason.

---

## 8. A working order for common tasks

**"Is anything broken?"**
`majnet status` → if a node is unreachable or failures are listed,
`majnet events --failed` → for one app, `majnet logs <p> <a> -c <class>`.

**"Why is this app misbehaving?"**
`majnet app <p> <a>` (declared vs running, per class) →
`majnet logs <p> <a> -c <class> -n 500` →
`majnet exec <p> <a> -c <class> -- <a diagnostic command>` →
`majnet sql <p> <a> -c <class> '<a read query>'` if it looks data-shaped.

**"Ship the current stable build to production."**
`majnet release list` / `majnet app` to confirm what is in stable →
`majnet deploy promote <p> <a>` (writes the overlay) →
`majnet deploy list <p>` (find the `env/production` render PR) →
have a human review it → `majnet deploy merge <p> <N>` →
`majnet deploy progress` to watch it land.

**"Cut a release."**
`majnet release drafts` → `majnet release draft show <p> <a>` to read the
generated changelog → adjust with `majnet release draft notes` if needed →
`majnet release draft submit <p> <a>` → `majnet release progress <p>`.

**"Roll back a bad deploy."**
`majnet deploy rollback <p>` reverts the ops repo head and propagates render
PRs. For an unhealthy container specifically, `majnet deploy restart` is faster
and changes nothing — the blue-green deploy keeps the old container alive
through a failed rollout, so a restart often restores service on its own.

---

## 9. Rules for an agent

- **Read before you write.** `majnet status` and the relevant `logs` first;
  almost every write here is visible to users within seconds.
- **Never pass `--yes` on a production command unless the human asked for that
  specific action in this conversation.** The prompt exists because production
  is the one class the platform itself gates.
- **Never `--write` a SQL statement on your own initiative.** Propose the
  statement, show what a read-only version returns, and let the human decide.
- **Never `--reveal` a secret unless asked**, and never echo a revealed value
  into a summary, a commit message, or a file.
- **Report what the platform said, not what you hoped.** If a command failed,
  quote its stderr. If identity was unresolved, say so instead of continuing.
- **Prefer `exec` over `shell`.** `exec` is scriptable, role-scoped and returns
  an exit code; `shell` needs platform admin, a real terminal, and records a
  transcript that a person will read.
- **One statement per `sql` call.** Multiple statements concatenate their result
  sets and the parsed shape stops being meaningful.
- **`--output json` for anything you parse.** Table output is elided by design.
