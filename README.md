# MajNet v2

A self-hosted deployment platform: **GitOps-driven**, built on **plain Docker** with static trust-zoned placement across three nodes, organized around **projects** — each project is its own GitHub organization, fully managed by the platform.

Two custom Rust services form the control plane:

- **GitHub Bot** (`crates/bot`) — the liaison. The only component talking to the GitHub and Tailscale APIs: org reconciliation, digest bumps, manifest rendering (render PRs onto `env/<class>` branches), membership + ACL sync, repo proxy for the reconciler, dashboard write API.
- **Reconciler** (`crates/reconciler`) — the orchestrator. Consumes rendered `env/*` branches, decrypts SOPS secrets with class keys, and converges each node's Docker API over WireGuard: blue-green deploys, per-project networks/ingress/DB provisioning, ephemeral GC.

**Credential isolation:** the bot holds the GitHub App key + Tailscale API key; the reconciler holds age keys + Docker mTLS certs. Disjoint powers.

📄 **Full design:** [docs/design.md](docs/design.md) · **Roadmap:** [docs/roadmap.md](docs/roadmap.md) · **Diagrams:** [docs/diagrams/](docs/diagrams/)

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
├── bootstrap/            # node bootstrap: Docker, WireGuard, roles, firewall
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
