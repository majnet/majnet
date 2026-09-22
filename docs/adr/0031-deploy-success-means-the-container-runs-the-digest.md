# 0031 — Deploy success should mean the container runs the intended digest

**Status:** proposed · **Date:** 2026-09-22 · relates to [0022](0022-observable-release-progress.md) (release progress), [0030](0030-image-reclamation.md) (the incident that exposed this), design §12 (convergence)

> Written up, not implemented. The fix needs a channel between the reconciler and
> the bot that does not exist today, and inventing one touches the credential
> isolation boundary — so it wants a decision, not a patch. ADR 0030 shipped
> without it.

## Context

During the 2026-09-22 disk outage on node `private`, a release cut against
`stable` reported **converged** while not one container had moved. Image pulls
were failing with `no space left on device`; the containers stayed on their old
images for hours. The release view stayed green throughout.

This is not a bug in the release code. It is what the code means.

`releases::record` advances a release to stage `tracked`, and
`Store::set_release_stage` maps `tracked` → `status = 'done'`. `tracked` fires
after `track_stable()` succeeds — and `track_stable` is a **git** operation: it
re-points `env/stable` at the newest release tag. So "done" means *the manifest
was committed*. Nothing in the path observes a node.

It cannot, today. The bot calls the reconciler for nothing, and the reconciler
calls the bot for exactly two things — `GET /api/registry-auth/{org}` (ADR 0012)
and repo snapshots. There is no reverse channel through which convergence could
be reported. The stages `committing → tagging → building → published → tracked`
(ADR 0022) are all bot-side by construction, and the pipeline ends one step short
of the thing the user actually asked for.

The reconciler is not the problem here — it knew. `converge_one` failed loudly on
every pull, recorded `FAILED: …` events and marked `deploy_progress` failed. Two
independent, correct views: one saying the manifest shipped, one saying the
rollout didn't. Nothing joined them, and the green one was the one being watched.

The general shape is worth stating plainly, because it will recur: **anything
that reports deploy success from git state alone reports green during a total
pull outage.**

## Decision (proposed)

**A release is not done when its manifest is committed. It is done when every
app it covers is observed running the digest that manifest pins.**

Add a terminal stage after `tracked` — call it `converged` — that only the
reconciler can satisfy, and let `tracked` mean what it actually is (the git step
completed). Three parts:

1. **The reconciler reports convergence.** After `converge_one` succeeds, POST
   the observed `(project, app, class, digest, commit)` to a new WG-internal bot
   endpoint. The reconciler already resolves `manifest.image_ref()` and already
   proves the container healthy before it records `deployed`; this publishes
   what it established rather than computing anything new. Fire-and-forget:
   convergence must not depend on the bot being up.

2. **The bot holds the release open until the digests match.** `release_progress`
   gains the expected digest per app. A release sits at `tracked` until a
   convergence report arrives carrying that digest, then moves to `converged`
   (`status = 'done'`). No report within a deploy budget → `failed`, with the
   reason.

3. **Both surfaces stop claiming more than they know.** The dashboard's
   `ReleaseSteps` stepper and `majnet` render `tracked` as "manifest committed,
   awaiting the fleet" rather than as a tick.

### Credential isolation

The reconciler already talks to the bot over the WireGuard-internal API, so this
adds no new trust boundary and no new credential on either side — the report
carries a digest and an app name, no secrets in either direction. That is worth
stating explicitly, because "the reconciler tells the bot something" reads at
first glance like a violation of §6. It is the same direction and the same
channel as `registry-auth`.

### Cheaper alternative, if the above is judged too large

Join the two views **in the CLI**, which already queries both the bot and the
reconciler in one command (`status` does exactly this today). Cross-check the
bot's release rows against the reconciler's `deploy_progress` for the same
app+class and refuse to print a bare "done" over a failed rollout. ~40 lines, no
new endpoint, no schema change.

It was rejected as *the* fix because it corrects one reader rather than the
claim: the dashboard, the bot's own API, and anything else consuming
`release_progress` stay git-green. It remains a reasonable stopgap if the full
signal is deferred again.

## Consequences

- **A release means what a person reading it thinks it means.** The common case
  — everything fine — is a few seconds slower to turn green, which is the
  correct price.
- **Two more failure modes become visible**: a converge that never ran (the node
  was unreachable) and one that ran and failed. Both currently render as success.
- **A deploy budget has to be chosen.** Too short and a slow-but-healthy rollout
  reports failure; too long and a wedged fleet looks pending for ages. The
  existing health-gate deadline is the natural anchor.
- **Reports can arrive for releases the bot has forgotten** — `release_progress`
  GCs terminal rows after 5 min (done) / 1 h (failed). An unmatched report must
  be dropped quietly, not treated as an error.
- **`converged` is per app.** A release covering several apps is done when the
  last one converges; a partial convergence must be visible as such, not rounded
  up to done or down to failed.

## Notes for whoever picks this up

- The reconciler's `deploy_progress` table is per `(project, app, class)` and
  already carries stage, status and detail — it is close to the right payload
  and may be worth reusing rather than defining a parallel shape.
- ADR 0030 added `deploy_progress_expire_stale`, so a stranded `active` row now
  resolves to `failed` after an hour rather than lingering. Anything that waits
  on `deploy_progress` can rely on rows reaching a terminal state.
- `info::capture` already scrapes the app's `/info` post-health-gate and records
  the reported version. If the goal is ever "running the intended *build*"
  rather than "the intended digest", that is the hook.
