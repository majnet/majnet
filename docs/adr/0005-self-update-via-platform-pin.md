# 0005 — Control-plane self-update via a version pin in the platform repo

**Status:** accepted · **Date:** 2026-07-07

## Context

Resolves open question §20.3 (reconciler self-update via ops repo vs manual
bump) — broadened to the whole control plane, since ADR 0004 settled that
bot, reconciler and setup all run as source-built systemd binaries on the
main node only. Today the only update path is re-running `install.sh` by
hand, which rebuilds whatever ref the operator passes; nothing records which
version *should* be running, and updating is invisible to the audit trail.

Constraints from the design:

- **Writes go through git (§6):** "which control-plane version runs" is
  platform state and should be a commit, not an SSH session.
- **Credential isolation (§6):** an updater must not grow a new credential
  class. The source repo is public, so fetching it needs no credentials at
  all; the platform repo (private) is only readable by the bot.
- The reconciler managing its own runtime invites circular failure
  (ADR 0004) — the updater must be outside all three services.

## Decision

**The desired control-plane version is a git ref pinned in `version.yaml`
at the root of the platform repo; a systemd timer on the main node converges
the installed binaries to it.**

- `version.yaml`: `control_plane: { ref: <branch | tag | full SHA> }`.
  The wizard seeds it with the **exact commit SHA the installer checked
  out**, so day one is already exact-pin, not track-a-branch. Bumping (or
  rolling back) the platform is a commit to the platform repo — reviewed,
  attributed, append-only, like every other state change.
- The bot gains `GET /api/platform/version` (WG-internal, next to the other
  platform endpoints): reads `version.yaml` off platform `main` and returns
  the ref. The updater never touches the GitHub API itself.
- **`majnet-update`** (shipped in `bootstrap/`, installed to
  `/usr/local/bin`): asks the bot for the pin — or takes an explicit ref
  argument as break-glass when the bot is down — then in `/opt/majnet`:
  `git fetch origin <ref>` → compare `FETCH_HEAD` to `HEAD` → if different,
  checkout, `cargo build --release`, install the three binaries **and
  itself**, restart the services. The whole body runs inside a `main()`
  function so overwriting the running script is safe.
- `majnet-update.timer`: hourly, `Persistent=true`, installed by
  `install.sh` alongside the service units.

## Consequences

- Version changes are audit-logged commits; `git log version.yaml` is the
  deploy history, and rollback is pinning an older ref.
- The updater holds **no credentials** (anonymous fetch of the public source
  repo; the pin comes from the bot over WG). If the source repo ever goes
  private, it will need a read-only deploy key — a new credential to place
  deliberately at that point, not now.
- Failure is safe by ordering: a failed fetch/build aborts before `install`,
  the old binaries keep running, and the timer retries next hour. Bot
  unreachable → the tick logs a warning and exits 0 (no flapping).
- Restarting services mid-flight is already a supported condition: the
  converge loop is idempotent and blue-green keeps the old container serving
  (ADR 0002); an interrupted render/org-sync pass re-runs.
- Scope limit: the updater replaces **binaries only**. Changes to systemd
  units, env files, or bootstrap steps still ship by re-running
  `install.sh` (idempotent) — kept out of the updater so a bad pin can never
  rewrite service configuration.
- Branch pins (e.g. `main`) remain possible and turn the timer into
  auto-deploy-on-push; tags or SHAs are the recommended discipline.
