# 0006 — TLS for the main node's public endpoints via Caddy

**Status:** accepted · **Date:** 2026-07-07

## Context

The main node exposes two plain-HTTP ports to the internet: the bot's
webhook listener (8080, permanent) and the setup wizard (7600, one-time,
closes at `/finish`). Webhook payloads are HMAC-signed and the wizard is
token-authed, but the wizard token, the GitHub App manifest redirect, and
webhook bodies all travel unencrypted. The design's public-surface table
(§7) predates phase 6 and lists 80/443 on the prod node only, behind
Cloudflare.

Options considered: Cloudflare proxied in front of main (reuses the prod
pattern, but couples control-plane reachability to Cloudflare and needs CF
configuration before the wizard can even run), or a local ACME-terminating
proxy.

## Decision

**Caddy on the main node, automatic ACME certificates, set up by
`install.sh` when a domain is provided** (`MAJNET_DOMAIN`, e.g.
`majnet.example.com`, with an A record pointing at the node *before*
install so the ACME HTTP challenge succeeds).

- Routing on one hostname: `/webhook` and `/healthz` → bot (127.0.0.1:8080);
  everything else → wizard (127.0.0.1:7600). After `/finish` the wizard
  listener is gone and non-webhook paths return 502 — harmless, nothing is
  supposed to call them.
- The installer records the choice in `node.env` (`MAIN_TLS_PROXY=1`);
  the firewall (step 40) then admits **80/443 instead of 8080/7600** on the
  main role, so the plain-HTTP ports are never publicly reachable on a TLS
  install. Without a domain, behavior is unchanged (8080/7600 open,
  documented risk).
- `install.sh` writes `MAJNET_PUBLIC_BASE_URL=https://<domain>` into
  `setup.env`; the wizard uses it for the GitHub App webhook URL and the
  printed wizard link. Without it, the existing
  `http://<public_host>:8080/webhook` form remains.

## Consequences

- Control-plane TLS has no third-party dependency: Caddy renews via ACME on
  the node. Cloudflare stays what it already was — the fronting for
  production apps on the prod node (§7), not for the control plane.
- One new package from Caddy's apt repository on the main node; Caddy runs
  as a plain systemd service, outside Docker — the control plane stays
  independent of the container runtime (same reasoning as ADR 0004).
- The origin IP of the main node is visible in DNS (no CF proxying). The
  node only answers 22/51820/80/443, SSH is key-only, and the webhook
  endpoint verifies HMAC on every request — accepted.
- The domain must exist and resolve before install; installs without a
  domain (IP-only) still work over plain HTTP, so nothing is lost for
  scratch environments.
