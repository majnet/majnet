# 0019 — Intra-project service discovery (stable network aliases)

**Status:** accepted · **Date:** 2026-07-19 · relates to [0018](0018-monorepo-apps.md), [0002](0002-blue-green-via-healthcheck-gated-routing.md)

## Context

Apps in a project share one per-project Docker network (`network_name(project)`),
and the reconciler already relies on stable container names for *platform*
services — a managed DB engine is a fixed `majnet-postgres` that apps resolve by
name (§15). But **app** containers are named
`<project>-<app>-<class>-<config-hash>`, and the hash changes on every
config/spec change. Blue-green (ADR 0002) deliberately churns that name on each
rollout. So an app container's only network DNS name is volatile — there is no
stable name by which one app can address a sibling.

That is fine for the common case (apps talk to the outside world through the edge
Traefik, and to their DB by the engine's fixed name). It breaks a multi-service
app that has **internal** app-to-app traffic — most concretely one that ships its
own reverse proxy fronting several sibling apps under a single public origin
(e.g. `sideline`: an nginx `proxy` app routes `/api`→`server`, `/docs`→`docs`,
`/`→`web`, and a `bot` app calls `server`). MajNet ingress is host-based per app
with no cross-app path routing, so preserving a single-origin design requires the
app's own proxy — which needs to resolve its siblings by a stable name.

## Decision

Give every app container a **stable DNS alias equal to its manifest `name`** on
the project network. In `deploy::container_spec` the `ContainerCreateBody` gains
a `networking_config` whose `endpoints_config` is keyed by the same network as
`host_config.network_mode` (Docker requires the single endpoint's key to match
the network mode), with `aliases: [manifest.name]`.

- The alias is the **manifest name**, which for a monorepo member is the
  repo-prefixed `<repo>-<leaf>` (ADR 0018) — e.g. `sideline-server`,
  `sideline-web`. Names are unique within a project, and all of a project's apps
  share one network, so a prefixed alias is collision-free across sibling
  monorepos; a bare leaf (`server`) would not be. Apps address siblings by the
  full name (`SERVER_HOST=sideline-server`).
- It is **stable across blue-green**: the alias tracks the manifest name, not the
  hashed container name, so a sibling's address does not change when it redeploys.
  During the brief blue-green overlap two containers transiently share the alias;
  Docker round-robins, and both are the same app mid-rollout, so this is benign
  (the old one is drained once the new one is healthy).
- `SPEC_VERSION` is bumped `2` → `3` so the whole fleet re-converges onto the
  aliased spec via normal health-gated blue-green rollouts, rather than silently
  keeping alias-less containers.

No manifest surface changes — the alias is derived, always on. Migration
containers are short-lived and not addressable, so they keep the network mode
without an alias.

## Consequences

- A multi-service app can keep a **single public origin**: one app (its own
  proxy) takes the ingress host and routes to siblings by stable name, preserving
  same-origin cookies / auth / OAuth without CORS or subdomain-splitting. This is
  the enabling capability for migrating `sideline` (proxy + server + web + docs +
  bot as a monorepo) onto MajNet.
- Inter-app calls that don't need the edge (a bot → its API) can go over the
  project network by name instead of hair-pinning through the public edge.
- The alias lives only on the project's own Docker network — no VPN, no public
  surface — consistent with the isolation model (§5: own Docker networks; DBs
  never on any VPN).
- One-time fleet-wide re-converge on deploy (the `SPEC_VERSION` bump), which is a
  normal blue-green rollout per app (zero-downtime, ADR 0002).
