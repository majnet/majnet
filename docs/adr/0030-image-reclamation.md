# 0030 — Image reclamation, and node disk as a first-class metric

**Status:** accepted · **Date:** 2026-09-22 · relates to [0017](0017-metrics-history-persistence.md) (metrics history), design §8 (ephemeral lifecycle), §12 (convergence, "deletions only when config is gone from git")

## Context

On 2026-09-22 node `private` — which hosts `stable`, `testing` and `ephemeral` —
reached **100% disk, 0 bytes free**. The failure cascaded exactly as a full disk
does: image pulls failed with `no space left on device`, `majnet-postgres` went
into recovery, `sideline-server-stable` crash-looped against `the database
system is in recovery mode`, and the bot and web stayed pinned on old images. A
release cut during the incident reported `env/stable` converged while no
container had moved at all.

`docker system df` on the node:

```
TYPE            TOTAL     ACTIVE    SIZE      RECLAIMABLE
Images            465        17    105.3GB   93.98GB (89%)
Containers         20        20      6.8MB        0B (0%)
Local Volumes       7         6     58.0GB   56.92GB (98%)
```

**465 images, 17 in use.** The reconciler's store held 158 ephemeral deploy
records across 35 PRs (#546–#586, 2026-07-23 → 2026-08-27), every one of them
closed or merged, with zero ephemeral containers running on any node.

### Why

Teardown was container-shaped. `gc::ephemeral_gc` (48 h grace) and the 7-day
hard TTL in `converge.rs` both reach `deploy::remove_app`, which removed the
app's containers and its per-PR network and stopped there. Nothing else
reclaimed images either: before this change the workspace contained **no image
removal of any kind** — `create_image` (pull) and `inspect_image`, and nothing
else.

That leaks against PR throughput rather than fleet size. A preview app is
`<app>-pr<N>` at a fresh commit, so every PR pulls digests no later PR will ever
request again — and `pull_image` short-circuits on `inspect_image` precisely
*because* digests are immutable, so a resident image is never reconsidered. 35
dead PRs × ~5 apps ≈ the 448 unused images. It fills; it never drains.

### There *was* a reaper. It ran nightly, and could not win.

`bootstrap/steps/30-docker.sh` installs `majnet-docker-prune.timer` — nightly at
04:30, `docker image prune -af --filter until=168h` — added 2026-09-07 by #155
after an earlier fill of this same node. It is installed, enabled, and healthy:
it last ran at 04:38 on the morning of the incident and took seven minutes, both
`ExecStart`s exiting 0.

It reclaims nothing that matters, because the retention window is set against the
wrong quantity. Measuring the node:

```
Images  330 total, 23 active, 89.67 GB, 78.01 GB reclaimable (86%)
  1 image built 2023-05-19 … 2026-07-23   (12 images older than two days)
169 images built 2026-09-21
149 images built 2026-09-22
```

**318 of 330 images were built within two days.** A 168 h window keeps all of
them by construction. Running `--filter until=72h` by hand on that node
reclaimed **0 B** while the disk sat at 96% and climbed ~4 GB/h; `until=4h`
reclaimed **73.58 GB** and took it to 51%.

So the node churns ~150 images/day, and the only reclamation it had retained a
week of them. A week of that churn cannot fit on a 157 GB disk that is already
giving 59 GB to volumes — and no fixed age cut can be correct, because the
quantity that varies is the *rate*, not the age. The nightly job is also blind to
pressure: it retained its full week at 100% disk exactly as it would at 10%.

Two conclusions, both load-bearing for the decision below:

- **Reclaim at teardown**, so a closed PR's images go when the PR closes rather
  than surviving a retention window sized for something else.
- **Make the backstop answer disk pressure, not the calendar** — reclaim *to a
  target*, oldest first, and treat age only as an anti-race guard.

### Why nothing warned

`NodeMetrics` carried `disk_images` (image-layer bytes from `docker df`), shown
only on the dashboard. Nothing anywhere reported the **filesystem**: not
`majnet status`, not `majnet metrics`, not the alert evaluator, which thresholded
CPU and memory only. Diagnosing this required a shell on the node. A node can go
from comfortable to wedged with every MajNet surface reporting green.

### The volume trap

`docker system df` called 56.92 GB of volumes (98%) reclaimable. That was an
artefact: Docker counts a volume reclaimable when no *running* container
references it, and the stable server and bot were crashed at that moment. Once
they recovered, **all 7 volumes were live data**, including the Postgres volume
already in recovery. The obvious-looking `docker system prune -a --volumes`
would have destroyed the stable database.

## Decision

**Reclaim images, on teardown and under disk pressure. Never volumes. And report
node disk everywhere a node is reported.**

### 1. Images only, structurally

`images.rs` calls exactly one destructive Docker API — `remove_image` — and
never `remove_volume` nor any `prune` that could reach one. This is a property
of the module, not a convention someone has to remember: automated reclamation
cannot name a volume, so it cannot repeat the trap above.

Both paths use `force: false`, which is the safety interlock: Docker refuses to
delete an image that any container references, running *or stopped*. A mistake in
our bookkeeping degrades into a logged 409, never into a live app losing its
image. `noprune: false` lets the now-parentless layers go — without it the image
record disappears and the disk stays exactly as full.

### 2. On teardown — precise and attributable

`remove_app` (and `gc_removed_apps`) record each container's `image_id`
**before** removing it, then reclaim those ids afterwards. Before the removal is
the only moment the image is attributable; after it, nothing in the system can
tell that image was ever ours.

Ordering matters and is load-bearing: the images pass runs last, because
`force: false` makes Docker refuse while the containers still exist. A refusal is
the *normal* case, not an error — two previews built from the same commit share a
digest, and the second teardown is what actually frees it.

This covers every teardown path: ephemeral grace GC, the 7-day hard TTL, purge,
rename, and a stable/production app whose config left git. It deliberately does
**not** cover blue-green: `converge_app` retires the old generation through
`remove_container_if_exists` directly, so a routine redeploy keeps the previous
digest resident and rollback stays local.

### 3. Under disk pressure — the backstop

A 10-minute loop reads the metrics snapshot the sampler already writes (no extra
probing — a backstop has no business adding load to a node that is out of disk)
and, for any node at or above `reclaim_disk_pct` (**85%**), reclaims images that
are simultaneously:

- **not in use** by any container, running or stopped;
- **not platform images** — engines, edge, ingress, the metrics/secrets helpers,
  collected from the modules that own them (`platform::platform_images`,
  `ingress::ingress_images`) so a version bump can't silently expose one. These
  are re-pulled from the deploy and metrics hot paths; churning them saves a few
  MB and costs a network round-trip on a node where pulling is what was already
  failing;
- **older than `reclaim_min_age_hours`** (**1 h**). This is a safety floor, not a
  retention policy: an image pulled seconds ago whose container does not exist
  yet is indistinguishable from garbage by reference count alone. It was 72 h in
  the first draft, copied from the operator's manual command — and the
  measurement above shows that would have reclaimed **0 B** on the node this ADR
  exists to save.

Candidates are then taken **oldest first, only until the node reaches
`reclaim_target_pct` (70%)**. Age is the best proxy available for "least likely
to be wanted again" — Docker records no last-used time — so the newest images,
which are the plausible rollback targets, are the last to go. Stopping at the
shortfall means a node 2 GB over target gives up 2 GB of cache, not all of it.
The 15-point gap below the 85% trigger is hysteresis, so a pass does not re-fire
on the next tick.

Below the threshold it does nothing at all, so a healthy node keeps its image
cache. Every pass that frees anything writes an `image-reclaim` event, so a disk
that drains overnight leaves a trail in `majnet events`. Being over threshold
with *nothing* eligible is logged as a warning: that is volumes or live images,
which reclamation cannot fix and an operator must.

The selection rules are a pure function (`images::reclaimable`) over plain data,
unit-tested without a daemon: every rule can only *protect*.

### 4. Node disk, reported

- The host probe reads `df -k /` alongside `/proc/meminfo` and `/proc/stat` in
  the same throwaway busybox container. The container's `/` is an overlay backed
  by the filesystem holding Docker's data root, so its numbers are the ones that
  matter — and are exactly what `majnet exec <app> -- df -h /` shows, which is
  how the outage had to be diagnosed.
- `NodeMetrics` gains `disk_total` / `disk_used`; `majnet status` and
  `majnet metrics` gain a **disk** column, flagged `!` at 85% and `!!` at 95%.
- `alert_disk_pct` (default **85**) joins the CPU and memory thresholds. Lower
  than those on purpose: CPU and memory spike and settle, a disk only fills.
- History (**V11**) records disk per sample, so the dashboard charts the trend.
  `disk_total = 0` means *not measured* — a pre-migration row, an older
  reconciler, or a probe that timed out — and every consumer renders it as a gap
  rather than as an empty disk.

### 5. Deploy-record retention

`V10__deploy_progress.sql` reasoned that one row per `(project, app, class)`,
overwritten each rollout, is "naturally bounded by the fleet size (no GC
needed)". True for stable and production, where the app set *is* the fleet. False
for ephemeral, whose app names carry a PR number: every PR mints keys no later
rollout overwrites. Hence 158 rows for long-dead PRs.

The sibling `app_info` table was already pruned on the GC pass;
`deploy_progress` simply never got added to it. It is now
(`deploy_progress_prune`).

Two further ways a row outlived its meaning, both of which made `majnet status`
quietly untrustworthy — a stale row is indistinguishable from a live one:

- **Stranded `active`.** `DeployTracker` writes `active` on entering a stage and
  `done`/`fail` on the way out, so a reconciler restarted mid-rollout leaves the
  row `active` forever. Now expired to `failed` after an hour
  (`deploy_progress_expire_stale`), keeping the stage it died at as the
  diagnostic. Two such rows were live on the fleet with no container behind them.
- **Immortal `failed`.** A `failed` row is only ever overwritten by another
  *rollout*. But a transient failure — a Docker timeout mid-`starting` — usually
  leaves the previous container already matching the desired spec, so every later
  pass reports "in sync", does no work, and writes nothing. Four apps read
  `failed` at **643–657 h old** while all four had been serving that entire time.
  In-sync is precisely the proof the failure is over (the running container
  matches git), and a genuinely stuck app never reaches it — convergence retries
  and fails again, refreshing the row. So converging in sync now retires a
  recorded failure (`deploy_progress_resolve_failed`), writing only when there is
  one to clear.

`majnet status --output json` compounded this by returning the whole table under
`deploys_in_flight`, a name that promises only live rollouts and which the table
renderer had always honoured. It now filters to `active` like the table, so a
reader of that field does not depend on the reconciler's bookkeeping to avoid
seeing history.

## Consequences

- **The leak is closed at the source** and backstopped for what teardown can't
  see: a pull that failed halfway, an app renamed away, a node unreachable when
  its PR closed, images predating this change.
- **Volumes are never touched by automation.** The destructive surface is one
  API call in one module.
- **90% is visible before 100%** — in the CLI, on the dashboard, in alerts, and
  as a chart with history behind it.
- **A redeploy still rolls back locally**: blue-green retirement is deliberately
  outside the teardown path, so the previous digest stays resident.
- **Costs a re-pull** when a preview is torn down and an identical digest is
  wanted again, and when the backstop fires on a node that later redeploys
  something it reclaimed. Both are bounded by the age grace and only happen on a
  node already under pressure.
- **`docker system df` will still report volumes as reclaimable** during an
  incident. That reading was wrong once and will be wrong again; it is an
  artefact of crashed containers, not a measurement. Reclaim images and let the
  apps recover.

## Operational note

`majnet-docker-prune.timer` remains installed and is now the second line, not the
first. Do not read its success as evidence the node is fine: on `private` it ran
nightly, exited 0, and reclaimed nothing of consequence for weeks. The useful
question is not *did it run* but *what is the image age distribution* —
`docker images --format '{{.CreatedAt}}' | cut -d' ' -f1 | sort | uniq -c`. If
the mass sits inside the retention window, the window is wrong for that node's
churn, and only pressure-triggered reclamation will hold it.

## Alternatives rejected

- **`docker image prune -af --filter until=72h` on a timer.** What an operator
  ran by hand, and it works — but it reclaims the transient busybox metrics
  helper on every pass (re-pulled into the probe's 6-second budget) and any
  platform image not currently running, including a DB engine on a node that has
  just run out of disk. Enumerating candidates ourselves costs one `list_images`
  and buys an explicit protected set and testable rules.
- **`docker system prune -a --volumes`.** Would have destroyed the stable
  Postgres volume during the incident. Not available to automation at any
  threshold.
- **Reclaiming the old generation during blue-green.** Tempting — it is the
  other place images go stale — but it would make every rollback a network
  round-trip. The old digest is exactly the thing worth keeping local.
- **`ImagePrune` with a `label!=` filter instead of an explicit keep-set.** App
  images are built by user repos; we don't control their labels, so there is no
  label that distinguishes ours from theirs.
- **Age-based sweeping on every converge pass, regardless of disk.** Keeps nodes
  permanently lean at the cost of steady re-pull churn on images that get reused.
  Disk pressure is the condition we actually care about.
- **A longer fixed age cut instead of a target.** This is what
  `majnet-docker-prune.timer` already does, and the measurement above is what it
  is worth on a node churning 150 images/day: any window long enough to be a
  useful rollback cache is also long enough to fill the disk. Whatever constant
  is chosen is wrong for some node, and wrong in the direction of an outage.
