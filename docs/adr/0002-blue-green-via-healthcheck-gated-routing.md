# 0002 — Blue-green via health-gated Traefik routing

**Status:** accepted · **Date:** 2026-07-03

## Context

The design (§3, §12.5) describes blue-green as "start new → health check →
flip Traefik label → stop old". Docker labels are immutable after container
creation, so a literal label flip would require recreating the container —
defeating the point.

## Decision

Lean on Traefik's docker-provider semantics: **containers with a failing or
pending HEALTHCHECK are not added to the load balancer**. The reconciler
starts the new container with its Traefik labels *and* a Docker HEALTHCHECK
already in place:

1. one-shot migration container must exit 0 (else abort before anything starts)
2. new container starts — Traefik sees it but won't route until healthy
3. reconciler polls `inspect` until `healthy` (bounded by
   `start_period + retries × (interval + timeout)`)
4. healthy → old containers stopped and removed (drain)
5. unhealthy/timeout → new container removed, old keeps serving, deploy
   recorded as FAILED with the causing commit

During step 4 both containers may serve briefly (Traefik round-robins two
healthy backends) — accepted; it's a graceful handover, not an outage.

Health command runs inside the container: `wget || curl` against
`127.0.0.1:<port><path>`. Images must ship one of the two (alpine's busybox
wget qualifies); manifests without `health:` get a 5-second
"still-running" gate only.

## Consequences

- No proxy restarts, no label mutation, no traffic loss on failed deploys.
- Apps without a real health endpoint get weaker guarantees — nudge every
  manifest to declare `health:`.
- The brief dual-serving window means deploys are not atomic per-request;
  if an app ever needs strict atomicity, move its router to Traefik's file
  provider and flip that file instead (revisit then).
