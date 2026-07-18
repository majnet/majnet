# 0014 — Managed database access via per-project Adminer

**Status:** accepted · **Date:** 2026-07-13 · Phases 1–3 done (prod, via the main-node Tailscale route); the reconciler-managed/per-project-tailnet-ingress form still waits on the private node

## Context

MajNet keeps databases off every network — engines listen only on per-project
Docker networks, and access is break-glass SSH + `docker exec` (§15). That is
correct for the data path but a poor operator experience: to inspect a
migrated app's database you SSH the node and drive `psql` by hand. Bringing the
`majksa-ops` apps over (each with a Postgres DB) made this the bottleneck.

We want **"open this app's database" as a button in the dashboard** that lands
in a web DB client (Adminer) already scoped to that app's data — without
weakening the network posture or the credential-isolation invariant (§6: the
bot/dashboard never hold DB credentials; only the reconciler derives them from
`db-master.key`).

Two facts constrain the design:

- **The dashboard cannot know an app's DB password.** They are
  `HMAC(db-master.key, …)`, and the master key is reconciler-only. So the
  dashboard can't mint an authenticated deep-link for a specific app user, and
  a per-app auto-login is impossible from the bot side.
- **The engine is shared per node.** One `majnet-postgres` per trust-zone node
  hosts every project's logical DBs. A single "superuser Adminer" would expose
  the whole engine to anyone who reaches it — too broad.

## Decision

### A second credential tier: a per-project human role

Alongside each app's own `(project,app,class)` role (unchanged — apps keep
authenticating as themselves), the reconciler provisions a **per-`(project,
class)` human role** `{project}_{class}` and `GRANT`s it membership in every
app role of that project. With Postgres role inheritance it therefore inherits
access to **all of the project's databases, and nothing else** — cross-project
isolation is untouched (distinct roles; prod/dev are separate engines).

- Password: `HMAC(db-master.key, "Postgres:project:{project}:{class}")` — a
  distinct input namespace from app passwords, still stateless/derived.
- Additive and non-breaking: existing app users and `DATABASE_URL`s are
  unchanged; this only adds a role + grants. `crates/reconciler/src/db.rs`
  (`project_role`, `derive_project_password`), provisioned inside `ensure`.
- Postgres only for now (the engine humans browse); other engines can follow
  if needed.

Trade-off: within a project, this human role can read every app's DB — i.e. the
human trust boundary is the **project**, matching the design's project-level
isolation (§15), one level coarser than the per-app app-role.

### Adminer is a per-project service with auto-login

Adminer is deployed **per project** on the project's existing tailnet ingress
(ADR 0013) — not per node — and configured (by the reconciler, which holds the
role password) to **auto-authenticate as `{project}_{class}`**. It is therefore
reachable only over the tailnet at `adminer.{project}.{base_domain}`, browser-
trusted via the ADR 0013 wildcard cert, and scoped to that project's DBs. No
password is ever typed, and nothing DB-related is exposed publicly. The trust
boundary is the tailnet + the project ACL (`tag:proj-<name>`), exactly as for
the project's app ingress.

### The dashboard button is a deep-link

On an app's page, each DB-backed class shows an **"Open in Adminer"** button
linking to:

```
https://adminer.{project}.{base_domain}/?pgsql=majnet-postgres&db={project}_{app}_{class}
```

Adminer is already authenticated (auto-login), so the link just selects the
app's database. The button reads a **configurable Adminer base-URL** so it can
point at an SSH-tunnelled `http://localhost:8081` in the interim, before the
per-project routed Adminer exists.

## Phasing

1. ✅ **Per-project human role.** `db.rs` provisions `{project}_{class}` +
   grants (Postgres); additive, non-breaking. Enables scoped Adminer login (vs
   superuser) even against the interim SSH-tunnel Adminer.
2. **Per-project Adminer service.** Reconciler-managed Adminer container on the
   project's tailnet ingress, auto-login plugin fed the project-role password,
   routed at `adminer.{project}.{base_domain}` (ADR 0013 wildcard). Needs the
   private node + `MAJNET_TAILNET`.
   - 🚧 **Partial (2026-07-18):** the reconciler now *manages* a single
     prod-level Adminer (`crates/reconciler/src/platform.rs::converge_adminer`) —
     `adminer:5` on a private `majnet-admin` network shared with postgres (so it
     resolves `majnet-postgres`), resource-capped (256M/0.5cpu), config-hash
     managed like edge-main. This replaces a hand-deployed, orphaned container
     (it had drifted onto a stale project network and could no longer reach
     postgres). Still **not routed** (kept off the public `edge` network) and
     **not auto-login yet** — tailnet ingress + the auto-login plugin remain.
3. ✅ **Dashboard button.** "Open in Adminer ↗" on DB-backed apps (app detail),
   deep-linking `https://adminer.prod.majksa.net/?pgsql=majnet-postgres&db={project}_{app}_{class}`.
   Prod-only for now (the only env with an Adminer); Adminer host hardcoded to
   `adminer.prod.majksa.net` until non-prod Adminers exist.

## Interim realisation (pre-private-node)

Phases 2–3 shipped without the per-project tailnet ingress by reusing the main
node: the prod Adminer runs on the prod node (auto-login plugin, WG-bound
`10.88.0.2:8081`), and the **main node's Caddy** serves `adminer.prod.majksa.net`
over Tailscale, reverse-proxying to it over WireGuard. TLS is an LE cert (lego
DNS-01) with a daily renewal timer. This is **manual, not GitOps-managed** — the
reconciler-owned, per-project-tailnet-ingress form (the clean end state) still
waits on the private node.

## Consequences

- **Credential isolation preserved:** the bot/dashboard still hold no DB
  passwords; the reconciler configures Adminer's auto-login. The dashboard
  button carries only a DB *name*, never a secret.
- **Network posture preserved:** engines still have no listener; Adminer is the
  only DB-adjacent surface and is tailnet-only, project-scoped.
- **Interim:** a manual SSH-tunnel Adminer (superuser) on the prod node covers
  today; the per-project routed Adminer supersedes it once the node is up.
- **Break-glass unchanged:** SSH + `docker exec` remains the ultimate path and
  the only way to reach the superuser.
