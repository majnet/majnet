# 0026 — Public ingress for non-production apps (Cloudflare Tunnel)

**Status:** accepted · **Date:** 2026-07-23 · relates to [0007](0007-reconciler-owned-platform-services.md), [0013](0013-auto-assigned-vpn-ingress-hosts-with-ssl.md); touches design §4 (topology), §7 (networking), §8 (environment classes)

## Context

Non-production classes (`stable`, `testing`, `ephemeral`) run on the **private node**
and are tailnet-only by static placement — no public HTTP (the firewall opens only 22
+ WireGuard; the project Traefik binds inside the Tailscale sidecar's netns; the bot
publishes only a DNS-only split-DNS wildcard). That is a deliberate trust-zone boundary,
but it blocks anything needing a real public origin — most concretely **third-party OAuth
callbacks** (Discord for sideline's `dev.sideline.cz`), which cannot call back to a
tailnet-only host.

We want a non-prod app to be **optionally** publicly reachable without turning the private
node into a public-facing box or opening its firewall.

## Decision

Add an opt-in **`ingress.public: true`**. When set on a non-production app, the reconciler
runs a **`cloudflared` sidecar** for that project on the private node. `cloudflared` dials
**outbound** to Cloudflare (no inbound port), and Cloudflare routes the app's public
hostname through the tunnel to the project's **existing** Traefik — which already routes
every app by `Host()`. So one **project-level** tunnel serves all of a project's public
hosts.

Credential isolation (hard invariant) is preserved: only the **bot** touches the Cloudflare
API. It provisions a remotely-managed tunnel + proxied DNS and hands the reconciler a
**scoped tunnel token**, exactly mirroring the Tailscale authkey flow. The reconciler never
holds Cloudflare API credentials.

### Mechanics

- **Manifest:** `Ingress.public: bool` (`crates/common/src/manifest.rs`),
  `#[serde(default, skip_serializing_if not)]` so existing manifests are byte-identical (no
  `config_hash` change / fleet recycle). `validate()` requires a `host` when `public`.
- **Bot:** `crates/bot/src/cloudflare.rs` gains `provision_tunnel(name, hosts)` →
  `ensure_tunnel` (find-or-create a `config_src: cloudflare` tunnel, fetch its token) +
  `configure_tunnel` (`PUT …/configurations` with ingress rules: each host →
  `https://127.0.0.1:443`, `originRequest.noTLSVerify: true`, + a 404 catch-all) +
  `ensure_dns_cname_proxied` (host → `<id>.cfargotunnel.com`, proxied). `account_id` is
  derived from the zone (`account.id`). A new WG-internal endpoint
  `POST /api/cloudflare-tunnel/{project}` (`crates/bot/src/cloudflared.rs`, body `{hosts}`)
  provisions and returns the token — same bind-address trust as `tailscale-authkey`.
- **Reconciler:** `crates/reconciler/src/ingress.rs` runs a `proj-{project}-tunnel`
  cloudflared container (`network_mode: container:proj-{project}-tailscale` → reaches Traefik
  on loopback; `TUNNEL_TOKEN` env; token-only run). `converge.rs` collects the public hosts
  **unioned across every rendered non-prod class** (`converge_project_ingress` +
  `public_hosts_union`) and passes them in; an empty set (or `public` turned off) tears
  the sidecar down. Config-hash includes the sorted host set, so host changes recreate + re-
  provision. `purge.rs` tears the tunnel down with the rest of the ingress stack.

  > ⚠️ **The union is load-bearing, and this originally shipped wrong.** The tunnel is
  > project-scoped, but the host set was first computed **per class**, inside the per-class
  > convergence. That was only correct while a project had a single non-prod class — the
  > constraint [0027](0027-per-class-network-isolation.md) then removed. With `CLASSES` ordered
  > `[testing, stable, production, ephemeral]`, sideline's `stable` created the
  > `dev.sideline.cz` tunnel and `ephemeral` (no public hosts → empty set → "public is off")
  > deleted it later in the same pass. `dev.sideline.cz` flapped between HTTP 200 and
  > Cloudflare **1033** on every loop for roughly a month, unnoticed because releases were
  > paused and nothing routinely probed the host. Ingress is now converged **once per
  > project**, before the classes that route through it.
- **TLS:** Cloudflare terminates real TLS at the edge; the loopback hop to Traefik uses the
  wildcard default cert (SNI won't match the custom host, hence `noTLSVerify`), and Traefik
  routes on the HTTP `Host` header.

### Trust-zone implication

Non-production code (auto-deployed, less-reviewed) becomes internet-facing for the opted-in
host. This is an explicit, per-app opt-in — the default stays VPN-only. No firewall change;
the node itself is never publicly bound (the tunnel is outbound-only).

## Prerequisites (operational)

- The bot's Cloudflare API token (`MAJNET_CLOUDFLARE_TOKEN`) must gain **Account → Cloudflare
  Tunnel → Edit** (+ Account read for `account_id`). Zone→DNS edit it already has.
- Any pre-existing manual DNS record for a host being tunneled must be removed — the tunnel
  manages its own proxied CNAME → `<id>.cfargotunnel.com`.

## Alternatives rejected

- **Public edge on the private node** (mirror `edge-main`): opens `:443` and makes the node —
  which also holds databases + other projects' non-prod — public-facing. Too large a blast
  radius.
- **Tailscale Funnel:** only serves `*.ts.net` on fixed ports; no clean custom-domain cert.

## Follow-ups

- Retract the stale proxied CNAME when a host is removed (config replace already updates
  routing; only the DNS record lingers).
- Pin `cloudflared` to a dated tag/digest (currently the `:latest` floating channel, like
  `tailscale:stable`).
