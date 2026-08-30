# MajNet v2 — monorepo

Self-hosted GitOps PaaS. **Read `docs/design.md` first** — it is the source of truth for all architecture decisions (final draft v4). `docs/roadmap.md` tracks phase status.

## Layout

- `crates/common` — shared types: manifest schema v1, `project.yaml` / platform config, `EnvClass`
- `crates/bot` — GitHub Bot: the **only** code allowed to touch GitHub/Tailscale APIs
- `crates/reconciler` — Reconciler: the **only** code allowed to touch node Docker APIs and age keys
- `crates/cli` — `majnet`, the client. Whole API from a laptop; authenticated by tailnet identity, never a token (ADR 0029). Agent-facing reference: `crates/cli/docs/agent-guide.md`
- `dashboard/`, `bootstrap/`, `templates/repo-templates/` — non-Rust components (see their READMEs)

## Hard invariants (from the design — do not violate)

- **Credential isolation:** bot = GitHub App key + Tailscale API key; reconciler = age keys + Docker mTLS certs. Never mix.
- **Writes go through git:** every *state* change is a commit/PR on an `ops` repo. The imperative exceptions are the ones that change nothing git owns — restart/redeploy-same-digest, and (ADR 0029) `exec` / `sql` — each role-gated and audited.
- **Static placement:** node follows from environment class (`production`→prod, `stable`/`ephemeral`→private). No scheduling logic.
- **Rendering never decrypts** secrets; the reconciler decrypts only at deploy time, into tmpfs — never env vars.
- **Archive, never delete** GitHub repos; container/stack deletions only when config is gone from git.
- Images are pinned **by digest**, never by tag.

## Commands

```sh
cargo build && cargo test && cargo clippy --workspace
shellcheck -x scripts/*.sh bootstrap/*.sh   # CI lints these too
```

Driving a live platform (or diagnosing one) goes through the CLI, not curl — the
dashboard answers an unauthenticated `/api` with HTTP 200 `text/html`, so a raw
request reads as an empty result:

```sh
cargo run -p majnet-cli -- --url http://<main-node> status
majnet agent-guide          # command semantics, roles, JSON shapes, the traps
```
