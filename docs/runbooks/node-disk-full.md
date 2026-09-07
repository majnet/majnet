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

`majnet metrics` does **not** report disk. Do not conclude a node is healthy from it.

`majnet exec` needs a running container, so pick one of the apps that is still up — the overlay it
reports is backed by the host's `/var/lib/docker` filesystem, which is the number you want.

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

`until=168h` keeps a week of images so `majnet deploy rollback` still has them locally. A bare
`image prune -af` deletes everything not currently running and turns a rollback into a registry
pull — or a failure, exactly when you need it least.

Once space is back, the reconciler converges on its own; `majnet events --project <p>` confirms it.

## Read `docker system df` before deleting

The split matters:

- **Images / build cache dominate** → normal accumulation. The scheduled prune
  (`majnet-docker-prune.timer`, installed by `bootstrap/steps/30-docker.sh`) handles it going
  forward; a one-off prune is enough now.
- **Volumes dominate** → something is leaking. Orphaned volumes from deleted ephemeral
  environments point at a reconciler bug, and no amount of image pruning fixes it. Investigate
  rather than prune.

## Why `private` fills first

Static placement (`docs/design.md`): `production` → **prod**, `stable` and `ephemeral` → **private**.
Every preview environment lands on `private`, so it accumulates images and volumes faster than
`prod` at the same release cadence. For comparison during the incident above, `prod` sat at 31G/503G
— 7%.

Watch it on the node whose apps release most often, and remember that a big image multiplies: one
app shipping four versions in a day at ~780 MB each is ~3 GB, and nothing reclaimed it before this
timer existed.
