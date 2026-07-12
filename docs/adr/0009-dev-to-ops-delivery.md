# ADR 0009 — DEV→OPS delivery: builds, releases, and the class gradient

**Status:** accepted (design; implementation phased)
**Date:** 2026-07-12

## Context

OPS is fully GitOps'd — the `ops` repo pins images by digest, the bot renders
`env/*`, the reconciler converges — but the **DEV→OPS handoff is a manual
`image: …@sha256:` edit**. There is no first-class path from an app's source
repository to its running environments.

We already have most of the machinery: multi-org webhook intake, digest bumps
(§11.4: stable auto-on-merge, production via promote), the render → `env/*` →
reconciler loop, the manifest's `migration` + `database` fields, GHCR-by-digest,
and the GitHub App's subscription to `push` / `pull_request` /
`registry_package` / `release` events. This ADR wires those into a delivery
pipeline.

## Decision

### Two tiers: **builds** vs **releases**

- **Builds** (PR, main) — *an image digest only*. Disposable, continuous, no
  ceremony. They feed the throwaway zones.
- **Releases** (git tags `vX.Y.Z`) — an *immutable, versioned descriptor*
  bundling the app image, the migration, and metadata. This is **the DEV→OPS
  contract**: versioned, shown on the dashboard, and promotable.

### The class gradient (adds `testing`)

```
PR opened ──────▶ ephemeral   per-PR preview, TTL-GC'd        (build)
merge to main ──▶ testing     continuous, latest main         (build)   ← NEW class
tag vX.Y.Z ─────▶ stable      versioned release, auto          (release)
promote ────────▶ production  a chosen release, admin-gated    (release)
```

- `EnvClass` gains **`Testing`**. Static placement holds:
  `testing`/`stable`/`ephemeral` → **private** node, `production` → **prod**.
- **`stable` is re-pointed from merge-driven to tag-driven** (amends §11.4:
  "merge → stable" becomes "merge → testing, tag → stable"). This makes
  `stable` genuinely stable — versioned — instead of "whatever's on main."
- Per-app overlays become `base` + `testing`/`stable`/`production`/`ephemeral`.

### The Release descriptor (GitHub Release)

On a tag, the app's CI pushes the artifacts by digest and publishes a **GitHub
Release** (`vX.Y.Z`) carrying `majnet-release.yaml`:

```yaml
version: v1.4.2
commit: 9f3c…
app: ghcr.io/acme/blog@sha256:…
migration:                                  # optional
  image: ghcr.io/acme/blog-migrate@sha256:… # optional; defaults to `app`
  command: ["dbmate", "up"]
```

Immutable, digest-pinned, human-visible on GitHub, and delivered to the bot via
the `release` webhook it already receives. The descriptor is a **release asset**
(not a committed file): CI only knows the digests *after* the build, i.e. after
the tag, so `app-release.yaml` builds → writes `majnet-release.yaml` → publishes
the Release with it attached. The bot downloads that asset off the webhook.

### Migrations (flexible — `migration = { image?, command }`)

The manifest's `migration` gains an optional `image`. This covers all three
shapes with one field:

- **App-image migration** — omit `image`; `command` runs in the app image
  (e.g. `["rails", "db:migrate"]`).
- **Separate migration image** — a distinct digest with its own tooling.
- **SQL migrations** — point `image` at a MajNet-provided standard runner
  (e.g. `dbmate`/`flyway`) and bundle the `.sql`; the runner applies them to the
  reconciler-provisioned DB.

The reconciler's existing §12 pre-rollout migration step runs `migration.image`
(default = app image) with `command`.

### App CI (reusable workflow, shipped in a starter template)

- MajNet owns a **reusable GitHub Actions workflow** (`build → push by digest →
  publish release`), just like the control-plane `images.yaml`.
- A **starter template repo** (`templates/repo-templates/`) wires that workflow
  in, so a new app starts delivery-ready; the pipeline updates centrally through
  the reusable workflow rather than drifting per-app.

## What this reuses vs adds

| Reused | Added |
|---|---|
| Webhook intake, render PRs, digest bumps, `env/*`→reconciler, the §12 migration step, GHCR-by-digest, the §9 production gate | `EnvClass::Testing`, `migration.image`, the Release descriptor + a bot **release store**, build-tier image bumps (PR/main → ephemeral/testing), a dashboard **Releases** tab, the reusable workflow + starter template |

## Component changes

- **common** — `EnvClass::Testing` (+ `node_role`, `ALL`, `as_str`,
  `env_branch`); `Migration.image: Option<String>`; a `Release` descriptor type.
- **bot** — on `release`: validate the descriptor → record it (available
  releases per app) → event. `tag → stable` auto-bumps the overlay; `promote`
  writes a chosen release's app+migration digests into `production.yaml`
  (existing digest-bump + render-PR path). On `push` to main/PR branches:
  build-tier digest bumps into `testing`/`ephemeral`.
- **reconciler** — run `migration.image` (default app image) with `command`.
- **dashboard** — per-app **Releases** tab: versions, artifacts, on-stable /
  on-prod markers, "Promote → production", and a diff vs the deployed release;
  testing/ephemeral show the current build.
- **templates/repo-templates** — the starter app repo + reusable workflow.

## Phasing

1. ✅ **`EnvClass::Testing`** — schema + render + converge + dashboard.
2. ✅ **Release descriptor + bot release-watch + store** — `Release` type,
   SQLite `releases` table, `release` webhook, `GET /api/releases/{org}/{app}`.
3. ✅ **Dashboard Releases tab + promote-from-release** — `migration.image`,
   `POST …/releases/…/promote/{version}`, per-app Releases panel.
4. ✅ **Reusable workflow + templates** — `.github/workflows/app-release.yaml`
   (build → push by digest → publish Release with `majnet-release.yaml` asset);
   `release.yaml` added to the web-app + rust-service templates. Bot reads the
   descriptor from the release asset.
5. **Build-tier wiring** — main/PR push → image bumps into testing/ephemeral.

## Open items

- **Release backfill** — a periodic reconcile listing releases + assets from
  GitHub, so a missed/out-of-order `release` webhook (e.g. asset attached just
  after publish) still populates the store. The webhook is the fast path.
- Build-tier trigger for `testing` (main push → bump `testing.yaml`): auto vs
  opt-in per app.
- Production promote: allow any release, or only newer-than-current?
- Descriptor provenance/signing (attestations) — later.
- `ephemeral` still builds per-PR; confirm it stays digest-from-PR-build (yes).
