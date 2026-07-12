# ADR 0012 — Node registry auth for private app images

**Status:** accepted (implemented)
**Date:** 2026-07-12

## Context

App source repos are private, so their GHCR packages
(`ghcr.io/<org>/<app>`) are private too. When the reconciler deploys an app, the
node's Docker must pull that image — but the reconciler holds **no GitHub/GHCR
credentials** by design (credential isolation, §6: reconciler = age keys +
Docker mTLS; bot = GitHub App + Tailscale). So the pull failed with
`unauthorized`. This first surfaced promoting a private app (`space-alert`) to
production; the demo apps and the control-plane images were all public, so nobody
had needed node→GHCR auth before.

## Decision

**The bot serves a GHCR pull credential; the reconciler uses it as Docker
registry auth.**

> **Note (implementation finding):** the first cut used the GitHub App's
> installation token (the App holds `packages: read`). **GHCR does not honor App
> installation tokens for package pulls** — the token authenticates but the pull
> 403/404s on the package. So the served credential is a **configured PAT**
> (`MAJNET_GHCR_TOKEN`, a fine-grained or classic token with `read:packages`),
> not the App token. The plumbing below is unchanged; only the credential
> differs. The App-token path remains as a fallback (enough for *public*
> packages, which need no real auth anyway).

- **Bot** exposes `GET /api/registry-auth/{org}` on the WG-internal listener,
  returning `{ username: "x-access-token", password: <MAJNET_GHCR_TOKEN, or the
  installation token if unset> }`. Trust is the WireGuard bind — same model as
  the snapshot API. (`proxy.rs`.) `MAJNET_GHCR_TOKEN` lives in the bot's env
  (`/etc/majnet/bot.env`), never on disk in the reconciler.
- **Reconciler**, before pulling a `ghcr.io/<org>/…` image, fetches that
  credential over WG and passes it to `create_image` as `DockerCredentials`
  (`deploy::ghcr_credentials` / `pull_image`). Non-GHCR images (public
  registries) get no auth; if the bot is unreachable the pull proceeds
  unauthenticated (fine for public images, and it just re-fails loudly for
  private ones on the next converge).

This keeps **credential isolation intact**: the reconciler never holds the App
key — only a short-lived, `packages:read`-scoped token obtained from the bot
over the trusted internal channel, used transiently for the pull.

## Alternatives considered

- **Make packages public** — defeats private source repos; per-app manual step.
- **Static PAT / `docker login` on the node at bootstrap** — a long-lived
  credential sitting on every node, outside the reconciler's managed flow and
  harder to rotate. The bot-minted short-lived token is strictly better.

## Consequences

- Private app images now pull on the nodes; no manual per-package visibility
  flips.
- One more reconciler→bot call on a cache-miss pull (images are digest-pinned
  and cached after first pull, so this is rare).
- The installation token is passed over WG (like the Tailscale authkey +
  snapshot token already are). It's short-lived and `packages:read`-scoped.
