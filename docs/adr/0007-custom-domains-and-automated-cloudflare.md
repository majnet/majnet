# 0007 — Custom domains + automated Cloudflare, reconciler-owned edge

**Status:** accepted · **Date:** 2026-07-07

## Context

Bringing the first public workload up (hello-world on prod) exposed two gaps:

1. **Platform services are deployed by hand.** The reconciler converges
   *project* apps (from each ops repo's `env/*` branches) but not the
   `platform/` services — `edge-main` (Traefik) and the DB engines. Those were
   a manual `docker compose` on the node. The design aspired to
   "reconciler owns edge-main" but never implemented it.
2. **Public serving is fully manual per domain.** Cloudflare DNS, the Origin
   CA cert, and the `Host()` route were all hand-done for `hello.majksa.net`.
   The platform must instead let a project expose an app on **arbitrary custom
   domains** (`app.majksa.cz`, …) with the whole Cloudflare + edge wiring done
   automatically as part of app onboarding.

Two facts about the reconciler shape the design:

- It reaches nodes **only via the Docker API** (bollard + mTLS over
  WireGuard). It has **no SSH** — so it can't push files the way the setup
  service does at enrollment.
- But it *can* place arbitrary files on a node without SSH: `secrets.rs`
  already runs a short-lived helper container and streams a tar via
  `put_archive` onto a host path. Origin certs and `edge-main`'s config reuse
  that mechanism.

And per-app Traefik routing **already exists**: `deploy.rs` emits
`Host()`/entrypoints/TLS/port labels whenever a manifest has `ingress`.

## Decision

### Domain model (manifest)

Extend `Ingress` in the schema — `host` stays the primary hostname, add an
optional `domains` list for additional public hostnames:

```yaml
ingress:
  host: app.majksa.cz     # primary public hostname
  port: 8080              # container port
  domains:                # optional additional hostnames (any zone)
    - www.majksa.cz
    - app.majksa.net
```

The app's full hostname set is `[host] + domains`. For the **production**
class these drive Cloudflare + edge routing; for stable/ephemeral, `host` is
the tailnet ingress name as today (no Cloudflare).

### Cloudflare is owned by the bot

The bot is already the sole external-API liaison (GitHub, Tailscale);
Cloudflare joins that class. A scoped token
`MAJNET_CLOUDFLARE_TOKEN` (permissions: **Zone → DNS → Edit** and **Zone →
SSL and Certificates → Edit** / Origin CA) goes in the bot's config. A new
`bot/src/cloudflare.rs` module owns all CF API calls. The reconciler never
sees the token — credential isolation (§6) holds.

Per production hostname, the bot ensures (idempotently):
- the hostname's **zone** exists on the account (matched by DNS suffix over
  the token's zones). Zone *creation* / nameserver delegation stays manual —
  the bot verifies the zone is present and errors clearly if not.
- a **proxied** DNS record for the hostname → the **prod node's public IP**
  (from `nodes.yaml`).
- the zone's SSL mode is **Full (strict)**.
- one **Cloudflare Origin CA certificate per zone**, covering `<zone>` and
  `*.<zone>` (reused by every app in that zone; regenerated before expiry).

### Origin certs flow through git, encrypted (the credential bridge)

The bot generates the Origin CA cert (CF API returns cert + private key), then
**encrypts the private key to the `age-production` recipient** (a *public*
key — no age private key needed by the bot) and **commits** it to the platform
repo:

```
platform/edge-main/certs/<zone>.crt        # public cert, plaintext
platform/edge-main/certs/<zone>.key.sops   # private key, age-encrypted
```

The **reconciler** — which holds `age-production` — fetches the platform
snapshot, decrypts each zone key (its existing SOPS path), and places
`cert.pem`/`key.pem` per zone on prod for Traefik. So the private key only
ever exists **encrypted in git** or **decrypted on the prod node**; the bot
touches Cloudflare + git, the reconciler touches age + Docker. Neither crosses
into the other's credential class.

### The reconciler owns edge-main (and the DB engines)

The reconciler deploys `platform/` services onto the node whose role matches
(§4): `edge-main` + production DB engines → **prod**; dev DB engines →
**private**; the dashboard stays a main-node bootstrap step. For each service
it, over the node's Docker API:
1. ensures the service's networks (e.g. `edge`),
2. delivers config files (`traefik.yaml`, dynamic config, per-zone certs) to a
   host path via the `secrets.rs` helper+`put_archive` mechanism,
3. creates/updates the container (blue-green-ish: recreate on config-hash
   change, like apps),
4. rebuilds Traefik's **dynamic TLS config** so each zone's SNI maps to its
   origin cert.

`edge-main` runs **Traefik v3.6+** (3.6 auto-negotiates the Docker API version;
Docker Engine 29 dropped the old 1.24 floor that ≤3.5 hard-codes — see the
live 525 debugging that led here).

### Production app routing

When the reconciler deploys a production app that has `ingress`, it (a) labels
the container with a `Host()` rule covering all its hostnames (already emitted;
extended to OR the `domains` list), and (b) **attaches it to the `edge`
network** so `edge-main` can reach it. Traefik already has the zone's origin
cert, so TLS + Full(strict) work with no per-app cert work.

### End-to-end onboarding flow

```
commit app manifest with ingress.host = app.majksa.cz
   → bot render (base ⊕ production overlay) → env/production
   → bot Cloudflare: ensure zone / proxied DNS → prod IP / Full-strict /
     origin cert (majksa.cz) → commit cert + age-encrypted key to platform
   → reconciler converge: decrypt key, place cert on prod, refresh Traefik TLS,
     deploy app on edge network with Host(app.majksa.cz) label
   → https://app.majksa.cz serves, end to end, no manual steps
```

## Phasing

1. **Reconciler-owned platform services.** Deploy `edge-main` (Traefik 3.6,
   `edge` network, origin-cert mount) + DB engines from the platform repo via
   the Docker API + helper file delivery. Removes the manual bring-up. Uses
   whatever origin cert is present initially.
2. **Manifest `domains` + routing.** Add `Ingress.domains`, validation;
   reconciler emits a multi-host rule and attaches prod apps to `edge`.
3. **Bot ↔ Cloudflare.** `cloudflare.rs`: token in config; ensure DNS +
   Origin CA cert + Full-strict per zone; commit certs (age-encrypted key);
   reconciler places them and refreshes Traefik TLS. Hook into render.
4. **Multiple zones / arbitrary custom domains.** Zone discovery/verification
   across the token's zones; per-zone origin cert + Traefik TLS entry;
   clear errors when a domain's zone isn't delegated to Cloudflare.

## Consequences

- **Credential isolation preserved and extended:** bot gains Cloudflare (an
  external API, its existing role); reconciler gains nothing new (it already
  decrypts age + drives Docker). The origin-cert private key crosses the
  boundary only as age-ciphertext in git.
- **GitOps intact:** every effect is a commit — the manifest domain, the bot's
  Cloudflare-cert commit, the render. `git log` remains the deploy history.
- **The reconciler grows a "platform services" convergence pass** alongside
  the per-project pass. It deploys compose-equivalent stacks by translating
  them to Docker API calls + helper-delivered config, not by shelling out to
  `docker compose` (which can't bind-mount client-side files onto a remote
  daemon).
- **Zone delegation stays manual** (registrar → Cloudflare nameservers); the
  bot verifies and reports, it can't delegate. Documented in onboarding.
- **hello-world** becomes redundant once a real app is onboarded; it stays as
  an optional smoke test and is no longer special-cased.
- Cloudflare API failures are non-fatal to app convergence where possible: a
  missing cert degrades to Traefik's default cert (a 525 at the edge) rather
  than blocking the deploy, and is retried next cycle — surfaced in the event
  log and dashboard.
