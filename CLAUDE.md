# MajNet v2 — monorepo

Self-hosted GitOps PaaS. **Read `docs/design.md` first** — it is the source of truth for all architecture decisions (final draft v4). `docs/roadmap.md` tracks phase status.

## Layout

- `crates/common` — shared types: manifest schema v1, `project.yaml` / platform config, `EnvClass`
- `crates/bot` — GitHub Bot: the **only** code allowed to touch GitHub/Tailscale APIs
- `crates/reconciler` — Reconciler: the **only** code allowed to touch node Docker APIs and age keys
- `dashboard/`, `bootstrap/`, `templates/repo-templates/` — non-Rust components (see their READMEs)

## Hard invariants (from the design — do not violate)

- **Credential isolation:** bot = GitHub App key + Tailscale API key; reconciler = age keys + Docker mTLS certs. Never mix.
- **Writes go through git:** every state change is a commit/PR on an `ops` repo. The single imperative exception is restart/redeploy-same-digest.
- **Static placement:** node follows from environment class (`production`→prod, `stable`/`ephemeral`→private). No scheduling logic.
- **Rendering never decrypts** secrets; the reconciler decrypts only at deploy time, into tmpfs — never env vars.
- **Archive, never delete** GitHub repos; container/stack deletions only when config is gone from git.
- Images are pinned **by digest**, never by tag.

## Commands

```sh
cargo build && cargo test && cargo clippy --workspace
```
