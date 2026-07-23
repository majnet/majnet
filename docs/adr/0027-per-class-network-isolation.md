# 0027 — Per-class network isolation for non-production classes

**Status:** accepted · **Date:** 2026-07-23 · relates to [0019](0019-network-aliases.md) (intra-project aliases), [0013](0013-auto-assigned-vpn-ingress-hosts-with-ssl.md); touches design §4 (topology), §7 (networking)

## Context

Sibling apps in a project resolve each other by a bare DNS alias = `manifest.name`
(`sideline-server`, ADR 0019), set on the single shared per-project network
`proj-<project>` (`deploy.rs::network_name`). The env class is encoded in the container
*name* and the DB *name* — but **not** in the network or the alias.

On the private node, `stable`, `testing`, and `ephemeral` all run. As long as a project has
only **one** non-prod class there, this is fine. The moment a project runs a **second**
non-prod class (e.g. standing up sideline's `testing` alongside `stable`), both classes'
`server` containers carry alias `sideline-server` on the same `proj-<project>` network →
Docker's embedded DNS returns both → app-to-app traffic round-robins across environments.
Verified live: `proj-sideline` had `sideline-sideline-server-stable` aliased `["sideline-server"]`;
a testing server would collide.

## Decision

Isolate app-to-app service discovery **per class**, additively, keeping the shared network for
infra reachability (which already works and must not change):

- **`proj-<project>` (shared, unchanged):** the per-project ingress Traefik sidecar
  (`ingress.rs`) and the shared managed DB engine `majnet-postgres` (`db.rs`) live here. Every
  app + migration **joins** it (apps via a post-start `connect_network`, like the existing
  `edge` join) for ingress routing and DB reachability (by container name). **No app alias
  here** → no cross-class name collision.
- **`proj-<project>-<class>` (new):** each app's **primary** network (`network_mode`), carrying
  its `manifest.name` alias. Siblings resolve each other **within their own class only**.

This works because Docker aliases are **per-endpoint**: an app aliased only on its class net
isn't discoverable by that alias on the shared net, so all classes coexisting on the shared net
collide only by unique container names (which nobody resolves). Ingress + DB reach apps over the
shared net exactly as before, so **connection strings, the DB engine, Adminer, and the `edge`
join are untouched**.

## Mechanics

- `deploy.rs`: add `class_network_name(project, class)` = `proj-{project}-{class}`; the app's
  `network_mode` + create-time alias move to it; after `start_container` the app also
  `connect_network(network_name(project))` (shared, no alias) — next to the prod-only `edge`
  connect. Migration containers stay on the shared net (they need only the DB engine).
- `converge.rs`: `ensure_network` now ensures both the shared and the per-class net (call site
  already runs per class).
- `ingress.rs`: the project Traefik gets `--providers.docker.network=proj-<project>` so it
  selects the app's **shared-net** IP (apps are now multi-homed); ingress spec salt bumped so
  existing ingresses recreate with the flag.
- `purge.rs` / `deploy::remove_network`: removes the shared net **and** every
  `proj-<project>-<class>` net.
- **`SPEC_VERSION` `"3"` → `"4"`**: the network name isn't in `config_hash`, so this is the lever
  that re-converges the fleet onto the new wiring (a one-time blue-green recreate of every app).

## Consequences

- Multiple non-prod classes can now coexist per project on the private node — unblocking a
  `testing` env alongside `stable`.
- **One-time fleet re-converge** on rollout (every app blue-green-recreates; zero-downtime per
  app). Per-project **tailnet** ingress recreates once (brief non-prod blip). **Prod public
  ingress (`edge-main`) is unaffected** — its own hash, not `SPEC_VERSION`-driven; prod apps keep
  their `edge` join. Precedent: `SPEC_VERSION "3"` was the ADR 0019 alias change.
- Migration containers can't resolve sibling aliases (they're only on the shared net) — accepted;
  migrations target the DB, not siblings.

## Alternatives rejected

- **Move everything (ingress sidecar + shared DB engine) to per-class nets:** would force
  multi-homing the single shared Postgres and each project's ingress sidecar across every class
  net — far more surface + runtime endpoints, for no benefit over keeping them on the shared net.
