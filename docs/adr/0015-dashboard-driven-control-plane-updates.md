# 0015 — Dashboard-driven control-plane updates

**Status:** accepted · **Date:** 2026-07-17 · extends [0005](0005-self-update-via-platform-pin.md), [0008](0008-ci-built-control-plane-images.md)

## Context

ADR 0005 made the desired control-plane version a git ref in the platform
repo's `version.yaml`, converged by a `majnet-update` systemd timer; ADR 0008
replaced the on-box `cargo build` with CI-built, digest-pinned images
(`control_plane: { ref, image, dashboard }`). Both left the *editing* of the
pin as a manual step — resolve the new digests by hand, commit `version.yaml`
directly (or via `gh api`), then wait for (or SSH in to trigger) `majnet-update`.
Three gaps:

- **No operator surface.** Bumping the control plane meant leaving the
  dashboard for the terminal — the one platform action with no UI.
- **Slow, opaque convergence.** The timer was hourly and `majnet-update` had no
  cheap "nothing changed" path, so a published pin could take up to an hour and
  gave no feedback while it rolled.
- **No running-version signal.** Apps report build metadata at `/info`
  (design §16), but the control plane itself did not, so nothing could tell
  *what is actually running* vs *what is pinned*.

Constraints carried from ADR 0005 still hold: writes go through git (§6), the
updater grows no new credential, and the reconciler must not manage its own
runtime.

## Decision

**Make the pin editable from the dashboard (still as a git commit), converge it
within ~1 min, and give the control plane its own `/info` build signal.**

- **Pin API (bot).** `GET /api/control-plane` returns the running pin, the
  latest available build (source `main` HEAD + its `sha-<HEAD>` image digests,
  resolved from GHCR — best-effort, degrades to `check_error`), the commits
  between them, and the `version.yaml` commit history. `PUT
  /api/control-plane/pin` (platform-admin) commits a new pin — explicit
  `{ref,image,dashboard}` to publish, or `{from_commit}` to roll back to a past
  `version.yaml`. Digest-pinning is enforced (tag-pinned images rejected). No
  new credential: the bot already holds the GitHub App key (commit) and the
  GHCR PAT (digest resolve, ADR 0012).
- **Running signal.** CI bakes `VERSION`/`GIT_COMMIT`/`BUILD_TIME` into both
  images (build-args, same as an app release). bot + reconciler share the image
  and both images are built from one commit, so the bot's baked commit
  describes the whole control-plane build. `GET /api/control-plane` reports it
  as `running` plus `converged` (running commit matches the pinned ref).
- **Faster, cheaper convergence.** `majnet-update` records the last converged
  pin (a stamp file) and no-ops cheaply when it is unchanged, so the timer can
  poll often; `majnet-update.timer` moves from hourly to `OnUnitActiveSec=30`.
- **Live progress.** The dashboard drives a rollout progress bar from
  `converged`; because it is a client-side SPA it survives the dashboard's own
  restart and rides the brief API blip (keep-previous-data + retry).

## Consequences

- The pin stays platform state committed through git — the dashboard `PUT` is
  just an App-authored commit, so `git log version.yaml` is still the deploy
  history and rollback is still pinning an older commit. Break-glass
  (`majnet-update <ref>`, direct `version.yaml` edit) is unchanged.
- The control plane is now self-updating from its own UI: the first
  dashboard-driven update ships the build that shows its own progress bar, so a
  build always demonstrates the *next* update's live status, not its own.
- **Not zero-downtime.** Convergence is `docker compose up -d`, which recreates
  bot + reconciler + dashboard (a few seconds of control-plane blip); deployed
  apps are unaffected (they run on their own nodes). This is a *recreate*, not
  blue-green — blue-green (ADR 0002) applies to app deploys, not the control
  plane. True zero-downtime would require rolling the bot (the API the progress
  bar reads) behind a proxy; deferred. The progress bar is built to ride
  through the blip instead.
- Polling every 30s adds a negligible `curl`+stamp-compare per tick; real work
  (git fetch, image pull, recreate) still happens only when the pin changes.
- `converged` is `null` for a build made before the `/info` metadata existed, so
  the UI cannot show a true rolling state for that one transitional update; a
  short optimistic window covers it.
