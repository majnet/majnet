# `majnet` — the MajNet CLI

Everything the dashboard does, from your laptop. Fleet status, logs, deploys,
releases, an interactive shell in a container, and SQL against an app's managed
database.

```
majnet status
majnet logs demo api -c stable --follow
majnet sql demo api -c stable 'SELECT count(*) FROM users'
majnet exec demo api -c stable -- ls -la /app
majnet deploy promote demo api
```

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/majnet/majnet/main/scripts/install-cli.sh | bash
```

Downloads the release binary for your platform (Linux and macOS, x86-64 and
arm64), verifies its checksum, and puts it on your PATH. Or, with a Rust
toolchain:

```sh
cargo install --git https://github.com/majnet/majnet majnet-cli
```

On a node it is already there — the control-plane image ships it, so
`docker exec majnet-bot majnet events --failed` works with no toolchain and no
WireGuard peer.

## Log in

```sh
majnet login      # finds the control plane on your tailnet and verifies identity
majnet whoami     # who the platform thinks you are, and your role per project
```

**There is no token.** Your identity is your Tailscale device: the control plane
sits behind a front door (`tailscale serve`, or the Caddy edge) that resolves
the calling tailnet IP to a login and injects it as a header the backends trust
(§16, ADR 0016). `people.yaml` maps that login to a GitHub user and the
platform-admin flag; each project's `project.yaml` carries the per-project role.
Both are edited in the dashboard, so **permissions granted in the UI are exactly
the permissions you have here** — nothing to issue, copy, rotate or revoke.

Your machine has to be on the tailnet (`tailscale up`), and your Tailscale login
has to be in `people.yaml`. That is the whole enrolment story.

`~/.config/majnet/config.yaml` holds URLs and defaults, and no credentials.

### Defaults, so commands stay short

```sh
majnet context set --project demo --app api --class stable
majnet logs                 # → demo / api / stable
majnet context list
```

Several contexts address several platforms; `--context <name>` picks one for a
single command, and `MAJNET_URL` overrides everything (handy in CI).

## The three identities, and why it matters

`majnet whoami` distinguishes three states that the HTTP API deliberately does
not:

| what you see | what it means |
|---|---|
| `you: majksa` | a named human — your project roles apply |
| `you: (unidentified …)` | no identity reached the API; calls are audited as `infra` and **bypass role checks** |
| an error about "the dashboard's SPA shell" | authentication failed and the dashboard answered `200 text/html` instead of `401` |

The middle row is the one to watch. It is *correct* on a node — the WireGuard
bind address is the credential (§12.1) — and wrong on a laptop, where it means
your identity is not getting through. The CLI never renders it as a name, and
never renders the third as an empty result.

## Command tour

Read (no side effects):

```sh
majnet status                              # the one-screen summary — start here
majnet events --failed                     # what recently broke
majnet nodes ; majnet metrics              # the fleet
majnet projects ; majnet apps demo         # the registry
majnet app demo api                        # one app: declared vs running, per class
majnet ps demo api -c stable               # containers
majnet logs demo api -c stable -n 500      # logs (--follow to tail)
majnet info demo api                       # what each env reported at /info
majnet manifest demo api                   # the committed YAML
majnet members demo                        # who has which role
majnet secrets demo api -c stable          # names (--reveal for values)
majnet db demo api -c stable               # which database, which engine, where
majnet control-plane status                # pinned vs published vs running
```

Deploy:

```sh
majnet deploy progress                     # rollouts in flight
majnet deploy restart demo api -c stable   # imperative; changes nothing in git
majnet deploy promote demo api             # writes the production overlay
majnet deploy list demo                    # the render PRs waiting
majnet deploy merge 42                     # merging the production one IS the deploy
majnet deploy rollback demo                # revert ops main; render PRs follow
```

Release:

```sh
majnet release drafts                      # everything waiting to be cut, fleet-wide
majnet release draft show demo api         # proposed version + generated changelog
majnet release draft submit demo api       # tag it and start the build
majnet release cut demo api --bump auto    # skip the draft; bump from commit messages
majnet release progress demo
```

Run things:

```sh
majnet exec demo api -c stable -- ls -la /app
majnet exec demo api -c stable --shell 'ps aux | head'
majnet shell demo api -c stable            # interactive (platform admin, recorded)
majnet shell --node private                # root shell on a node's host
majnet sql demo api -c stable --tables
majnet sql demo api -c stable 'SELECT * FROM users LIMIT 5'
majnet sql demo api -c stable              # a small REPL, on a terminal
```

## Writes go through git

Almost nothing here deploys anything directly, and that is the design: every
state change is a commit on an `ops` repo, and the reconciler converges from
git. `promote` writes the production overlay and a **gated render PR follows**;
merging that PR is the deploy. `rollback` reverts the ops repo's head.

Three commands are imperative, because they change nothing git owns:
`restart`, `exec` and `sql`. Each is role-gated and writes an audit event under
your name.

## Roles

| | production | stable / testing / ephemeral |
|---|---|---|
| read (logs, ps, secrets, sql) | project **admin** | project **developer** |
| restart, exec | project **admin** | project **developer** |
| SQL `--write` | project **admin** | project **admin** |
| promote, merge, rollback, release cut | project **admin** | — |
| `shell`, `control-plane pin` | **platform admin** (named) | **platform admin** (named) |

Production-touching commands prompt for confirmation; `--yes` skips it, and
without a terminal they refuse rather than assume.

## SQL, honestly

`majnet sql` runs the engine's own client inside the engine container,
authenticating as **the app's own database role** — never the superuser. A query
gets exactly the privileges the app has.

Read-only is the default. Without `--write` the statement runs in a read-only
transaction, which stops a mistyped `UPDATE`. It is **not a sandbox**: anyone
able to run SQL at all can open another transaction. The real boundary is the
role check above and the audit trail. Postgres and MariaDB return rows; Valkey
and MongoDB take their own client's commands and return raw text.

## Scripting and agents

- `--output json` prints the control plane's JSON unmodified. When stdout is not
  a terminal that is already the default, so a piped `majnet` emits JSON.
- `majnet exec` exits with the **container's** exit code and passes its
  stdout/stderr through, so it composes in shell conditionals.
- `majnet agent-guide` prints a full machine-readable reference — command
  semantics, roles, JSON shapes, and the answers that look like success but are
  not. `majnet agent-guide --install` drops it into
  `.claude/skills/majnet/SKILL.md` so an agent working in a repo picks it up.
- `majnet completions bash|zsh|fish|…` prints a completion script.

## Reaching the control plane

Default is through the dashboard origin, which is where identity is injected.
`--direct` talks to the WireGuard-internal listeners instead — only routable
from a node or an enrolled peer, and it sends **no identity**, so every role
check passes and actions are audited as `infra`. That is break-glass; the
in-image CLI on a node is configured that way on purpose. `majnet shell` refuses
`--direct` outright, because a terminal session has to be attributable.

## Environment

| variable | effect |
|---|---|
| `MAJNET_URL` | control-plane origin (overrides the context) |
| `MAJNET_BOT_URL`, `MAJNET_RECON_URL` | WG-internal listeners; implies direct mode |
| `MAJNET_PROJECT` | default project |
| `MAJNET_CONFIG` | config file path |

## See also

- `docs/adr/0029-installable-cli.md` — why the CLI authenticates this way, and
  why `exec`/`sql` are gated where they are
- `crates/cli/docs/agent-guide.md` — the agent-facing reference (`majnet agent-guide`)
