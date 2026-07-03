# MajNet v2

A self-hosted deployment platform: **GitOps-driven**, built on **plain Docker** with static trust-zoned placement across three nodes, organized around **projects** — each project is its own GitHub organization, fully managed by the platform.

Two custom Rust services form the control plane:

- **GitHub Bot** (`crates/bot`) — the liaison. The only component talking to the GitHub and Tailscale APIs: org reconciliation, digest bumps, manifest rendering (render PRs onto `env/<class>` branches), membership + ACL sync, repo proxy for the reconciler, dashboard write API.
- **Reconciler** (`crates/reconciler`) — the orchestrator. Consumes rendered `env/*` branches, decrypts SOPS secrets with class keys, and converges each node's Docker API over WireGuard: blue-green deploys, per-project networks/ingress/DB provisioning, ephemeral GC.

**Credential isolation:** the bot holds the GitHub App key + Tailscale API key; the reconciler holds age keys + Docker mTLS certs. Disjoint powers.

📄 **Full design:** [docs/design.md](docs/design.md) · **Roadmap:** [docs/roadmap.md](docs/roadmap.md) · **Diagrams:** [docs/diagrams/](docs/diagrams/)

> This repo is the platform **source code only**. Live platform config lives in GitHub: the `majksa-platform/platform` repo (nodes, people, project registry — seeded from [`platform-seed/`](platform-seed/)) and one `ops` repo per project org.

## Quick start

### Hacking on the platform

Everything you need comes from **nix + direnv** ([hook direnv into your shell](https://direnv.net/docs/hook.html) first):

```sh
git clone git@github.com:maxa-ondrej/majnet.git && cd majnet
direnv allow          # builds the dev shell: Rust, clippy, rust-analyzer, sops, age, plantuml
cargo test --workspace && cargo clippy --workspace
```

Then prove the core actually works — the smoke test runs the reconciler's full loop (converge → SOPS secret on tmpfs → blue-green → GC) against your **local Docker daemon**, no servers or GitHub needed:

```sh
scripts/smoke-test.sh
```

### Installing the platform (operators)

The end goal is a Coolify-style one-line install (roadmap phase 6). Until then, bringing up a real installation is the phase-0/1 manual path:

1. **Nodes** — provision 3 Debian machines (main / prod / private) and run [`bootstrap/`](bootstrap/README.md): WireGuard mesh, Docker APIs on WG + mTLS, per-zone firewalls.
2. **GitHub** — create the `majksa-platform` root org, push [`platform-seed/`](platform-seed/README.md) as its `platform` repo, register the GitHub App per [`crates/bot/README.md`](crates/bot/README.md).
3. **Keys** — `age-keygen` the two class keys (`age-stable.key`, `age-production.key`) + `openssl rand -hex 32 > db-master.key` into the reconciler's key dir.
4. **Control plane** — run `majnet-bot` and `majnet-reconciler` on the main node (env-var tables in the two crate READMEs), dashboard via [`dashboard/`](dashboard/README.md).
5. **First project** — create a project org, install the App on it, add one line to `projects.yaml`. The bot materializes everything else.

Day-2 operations live in [`docs/runbooks/`](docs/runbooks/).

## Topology

| Node | Trust zone | Runs |
|---|---|---|
| **main** | control plane | bot, reconciler + DB, dashboard, Dozzle, Beszel |
| **prod** | public workloads | `edge-main` (Traefik), production apps + DBs |
| **private** | internal workloads | per-project ingresses, stable + ephemeral apps, dev DBs |

Environment classes: `production` (public, gated by a reviewed render PR), `stable` (VPN, auto-deploy), `ephemeral` (VPN, PR-scoped, TTL'd).

## Repository layout

```
majnet/
├── Cargo.toml            # Rust workspace
├── crates/
│   ├── common/           # shared types: manifest schema, project + platform config
│   ├── bot/              # GitHub Bot (liaison)
│   └── reconciler/       # Reconciler (orchestrator)
├── dashboard/            # web UI over reconciler (reads) + bot (writes)
├── bootstrap/            # node bootstrap: Docker, WireGuard, roles, firewall (bash, Debian)
├── platform-seed/        # initial content for the majksa-platform/platform repo
├── templates/
│   └── repo-templates/   # app repo templates (GHA workflow, branch protection)
└── docs/
    ├── design.md         # the design document (source of truth)
    ├── roadmap.md        # phased roadmap + status
    ├── adr/              # architecture decision records
    ├── diagrams/         # PlantUML + Mermaid sources
    └── runbooks/         # operational runbooks
```

Note: this monorepo holds the **platform source code**. Live platform *config* lives in GitHub — the root `majksa-platform/platform` repo (nodes, people, project registry) and each project org's `ops` repo. See design doc §2 and §10.

## Development

The toolchain is provided by **nix + direnv** (`flake.nix` + `.envrc`): Rust (rustc, cargo, clippy, rustfmt, rust-analyzer), plus `sops`, `age`, and `plantuml`. With [direnv hooked into your shell](https://direnv.net/docs/hook.html), `cd` into the repo and run `direnv allow` once — the environment loads automatically from then on (cached via nix-direnv).

```sh
cargo build            # build the workspace
cargo test             # run tests
cargo run -p majnet-bot
cargo run -p majnet-reconciler
```

## Status

Pre-implementation — structure scaffolded from the final design draft (v4, 2026-07-03). Currently in **Phase 0 — Foundations**. See [docs/roadmap.md](docs/roadmap.md).
