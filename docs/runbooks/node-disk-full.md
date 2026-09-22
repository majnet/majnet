# Node disk full

A node at 100% cannot write, so containers die and **every converge fails**. The apps that are
already up often stay up, which makes it look like a deploy problem rather than a disk problem.

Observed shape: `private` at **155.9G / 156.2G, 0 bytes available**. Two apps had no running
container, three others were healthy, and every converge since a timestamp minutes earlier had
failed.

## Recognising it

The tells, in the order they usually appear:

```sh
majnet logs <project> <app> -c stable
#   Error grabbing logs: write /var/lib/docker/tmp/…: no space left on device
```

That message is the giveaway — **log retrieval breaks before most apps do**, because grabbing logs
needs scratch space. It also means the tool you would reach for next is the one that just failed.

```sh
majnet ps <project> <app> -c stable        # "none" for some apps, healthy for others
majnet exec <project> <app> -c stable -- df -h /      # use an app that IS still running
majnet events --failed --project <p>       # a wall of `converge` failures, all one node
```

`majnet status` and `majnet metrics` report node disk (**ADR 0030**) — a `disk` column,
flagged `!` at 85% and `!!` at 95%. A node whose disk column is blank is one the probe could not
measure (unreachable, or the probe timed out); that is not the same as healthy. Alerts fire at
`alert_disk_pct` (85% by default), and the dashboard charts the trend.

`majnet exec` needs a running container, so pick one of the apps that is still up — the overlay it
reports is backed by the host's `/var/lib/docker` filesystem, which is the number you want.

## Check the reapers before reaching for a manual prune

Two things should already be reclaiming, and the useful question is which one stopped:

```sh
majnet events --project <p> | grep image-reclaim   # the reconciler's backstop (>85% disk)
majnet shell --node <node>                         # then, on the host:
systemctl status majnet-docker-prune.timer         # the nightly node timer, 04:30
systemctl list-timers majnet-docker-prune.timer    # when did it LAST run?
```

**A green timer is not evidence the node is fine.** On `private` during the 2026-09-22 incident it
was installed, enabled, and had run that morning for seven minutes with both `ExecStart`s exiting
0 — while the disk went to 100%. Check what it can actually reach:

```sh
docker images --format '{{.CreatedAt}}' | cut -d' ' -f1 | sort | uniq -c   # age distribution
```

If the mass of images sits **inside** the timer's `until` window, the window is wrong for this
node's churn and the timer will never help. `private` had **318 of 330 images built within two
days**; `--filter until=72h` reclaimed **0 B** at 96% disk, and `until=4h` reclaimed **73.58 GB**.
Pick the window from that histogram, not from habit — and remember the reconciler's backstop
(ADR 0030) reclaims to a *target* rather than by age, so it does not have this failure mode.

## Reclaiming space

```sh
majnet shell --node <node>     # host shell; platform admin, records a transcript
docker system df               # look BEFORE deleting — see below
docker builder prune -af
docker image prune -af --filter until=168h
docker container prune -f --filter until=168h
```

**Never `docker system prune --volumes`.** These nodes host the managed databases, and that flag
removes volumes no *running* container claims — a Postgres stopped mid-deploy qualifies. It is the
one flag that turns a cleanup into data loss.

`until=168h` keeps a week of images so `majnet deploy rollback` still has them locally, and a bare
`image prune -af` deletes everything not currently running and turns a rollback into a registry
pull. **But on a fast-churning node a week is unaffordable** — see the histogram above, and pick
the shortest window that actually frees space. `until=4h` still leaves today's rollback targets.

Once space is back, the reconciler converges on its own; `majnet events --project <p>` confirms it.

## Read `docker system df` before deleting

The split matters:

- **Images / build cache dominate** → normal accumulation, *if* the scheduled prune
  (`majnet-docker-prune.timer`, installed by `bootstrap/steps/30-docker.sh`) can reach it. Check
  the age histogram above before assuming it can: its fixed `until` window is sized in days and a
  busy node can outrun it entirely.
- **Volumes dominate** → **do not believe it mid-incident.** Docker counts a volume reclaimable
  when no *running* container references it, so every volume belonging to a crashed app is
  counted. During the 2026-09-22 incident it reported 56.92 GB / 98% reclaimable; once the stable
  server and bot recovered, **all 7 volumes were live data**, including the Postgres volume that
  was in recovery. Restore the apps first, then re-read `docker system df`. If volumes genuinely
  dominate with everything healthy, that is a reconciler bug to investigate — never a prune.

## Why `private` fills first

Static placement (`docs/design.md`): `production` → **prod**, `stable` and `ephemeral` → **private**.
Every preview environment lands on `private`, so it accumulates images and volumes faster than
`prod` at the same release cadence. For comparison during the incident above, `prod` sat at 31G/503G
— 7%.

Watch it on the node whose apps release most often, and remember that a big image multiplies: one
app shipping four versions in a day at ~780 MB each is ~3 GB.

Preview environments are the dominant term, and they now reclaim their own images as their
containers go (**ADR 0030**): closing a PR frees the digests it pulled, instead of leaving them for
a nightly timer to find a week later. The nightly timer and the reconciler's disk-pressure backstop
are what catch the rest.
