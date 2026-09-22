//! Image reclamation — the disk half of teardown.
//!
//! Teardown used to be container-shaped: `deploy::remove_app` dropped an app's
//! containers and (for a preview) its per-PR network, and stopped there. The
//! images those containers had pulled stayed on the node forever. Nothing else
//! reclaimed them either — before this module the workspace contained no image
//! removal of any kind, only `create_image` (pull) and `inspect_image`.
//!
//! That leaks in proportion to PR throughput rather than to fleet size. Every
//! preview app is `<app>-pr<N>` (`deploy::preview_split`) at a fresh commit, so
//! every PR pulls image digests no later PR will ever ask for again — and
//! `deploy::pull_image` short-circuits on `inspect_image` precisely *because*
//! digests are immutable, so a present image is never re-examined. 35 closed PRs
//! × ~5 apps is ~450 images nothing has a reason to touch. On node `private`
//! that reached 465 images / 17 in use / 94 GB reclaimable, filling the disk to
//! 100%: pulls failed with `no space left on device`, `majnet-postgres` dropped
//! into recovery, and `stable` crash-looped behind it.
//!
//! **Images only.** This module calls exactly one destructive Docker API,
//! `remove_image`, and never `remove_volume` or a `prune` that could reach one.
//! That is a deliberate structural guarantee, not a convention: during the
//! incident `docker system df` reported 56.9 GB of volumes "reclaimable" (98%),
//! which was an artefact of Docker counting a volume reclaimable whenever no
//! *running* container references it — and the stable server/bot were crashed at
//! that moment. Once they recovered, every one of the 7 volumes was live data,
//! including the Postgres volume already in recovery. A `docker system prune -a
//! --volumes` would have destroyed the stable database. Automated reclamation
//! must never be able to make that mistake, so it cannot name a volume at all.
//!
//! Two paths, both `remove_image` with `force: false`:
//!
//! 1. **On teardown** (`release`) — reclaim exactly the images the app that just
//!    died was running. Precise, attributable, zero collateral.
//! 2. **Under disk pressure** (`reclaim_loop`) — a backstop for everything path 1
//!    can't see: a pull that failed halfway, an app renamed away, a node that was
//!    unreachable when its PR closed, images predating this module.
//!
//! `force: false` is the safety interlock and the reason both paths are
//! best-effort: Docker refuses to delete an image any container still references
//! — running *or* stopped — so a mistake in our bookkeeping degrades into a
//! logged 409, never into a live app losing its image.

use anyhow::{Context, Result};
use bollard::query_parameters as qp;
use bollard::Docker;
use majnet_common::platform::NodesFile;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use crate::AppState;

/// How often the backstop looks for a node under disk pressure. Long, because
/// it is a backstop: teardown reclamation is what keeps a healthy fleet lean,
/// and a node does not cross a disk threshold between two ten-minute ticks
/// without something already having gone wrong.
const RECLAIM_INTERVAL: Duration = Duration::from_secs(600);

/// Disk usage (%) at which the backstop starts reclaiming on a node. Override
/// with the `reclaim_disk_pct` config key. Below this it does nothing at all —
/// a healthy node keeps its image cache, so redeploys and rollbacks stay local.
const DEFAULT_DISK_PCT: f64 = 85.0;

/// Reclaim down to this much disk usage (%) once triggered, oldest image first.
/// The gap below `DEFAULT_DISK_PCT` is hysteresis: freeing to just under the
/// trigger would re-fire on the next tick. Override with `reclaim_target_pct`.
const DEFAULT_TARGET_PCT: f64 = 70.0;

/// Safety floor: an unused image younger than this is never reclaimed, because
/// an image pulled seconds ago whose container does not exist yet is
/// indistinguishable from garbage by reference count alone. An hour is far
/// beyond any pull→create window and is deliberately **not** a retention policy.
///
/// It was 72 h, taken from the `until=72h` an operator ran by hand during the
/// incident. Measuring the node showed that to be worthless here: 318 of its 330
/// images had been built within two days, so `until=72h` reclaimed **0 B** while
/// the disk sat at 96% and climbed ~4 GB/h. An age cut cannot bound a disk when
/// the churn rate is the thing that varies — hence reclaiming to a *target*
/// (below) and keeping age only as the anti-race guard it should always have
/// been. Override with `reclaim_min_age_hours`.
const DEFAULT_MIN_AGE_HOURS: i64 = 1;

/// What one reclamation pass recovered.
#[derive(Debug, Default, PartialEq)]
pub struct Reclaimed {
    pub images: usize,
    pub bytes: i64,
}

impl std::fmt::Display for Reclaimed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} image(s), {:.1} GB",
            self.images,
            self.bytes as f64 / 1_000_000_000.0
        )
    }
}

/// The subset of `ImageSummary` the selection rules actually read — so the
/// rules below are a pure function over plain data and can be tested without a
/// Docker daemon.
#[derive(Debug, Clone)]
pub struct ImageFacts {
    pub id: String,
    pub repo_tags: Vec<String>,
    /// Image creation time, unix seconds (not last-used — Docker does not
    /// record that, which is why `in_use` does the real protecting).
    pub created: i64,
    pub size: i64,
}

/// Which images may be reclaimed, given what is running and what the platform
/// needs. Deliberately conservative — every rule here can only *protect*:
///
/// - **In use** by any container, running or stopped: the image is live, or is
///   one `docker start` from live. Docker would refuse anyway; not asking keeps
///   the logs honest about what was actually a candidate.
/// - **Protected**: an image the platform itself runs (engines, edge, ingress,
///   the metrics/secrets helpers). These are re-pulled from the deploy and
///   metrics hot paths, so churning them saves a few MB and costs a network
///   round-trip at exactly the wrong moment — including on a node whose disk
///   just filled, where a pull is what was failing in the first place.
/// - **Younger than `min_age_secs`**: an image pulled moments ago has no
///   container yet. Without this the backstop could reclaim an image out from
///   under a rollout that is mid-flight.
pub fn reclaimable<'a>(
    images: &'a [ImageFacts],
    in_use: &BTreeSet<String>,
    protected: &BTreeSet<String>,
    now: i64,
    min_age_secs: i64,
) -> Vec<&'a ImageFacts> {
    images
        .iter()
        .filter(|img| !in_use.contains(&img.id))
        .filter(|img| !img.repo_tags.iter().any(|t| protected.contains(t)))
        .filter(|img| now - img.created >= min_age_secs)
        .collect()
}

/// Reclaim specific images by id — the teardown path, called once an app's
/// containers are gone.
///
/// Best-effort and order-dependent: the caller must have removed the containers
/// first, because `force: false` makes Docker refuse while any of them still
/// references the image. A refusal is the normal case, not an error — two
/// previews built from the same commit share a digest, and the second teardown
/// is what actually frees it — so failures are logged at debug and swallowed.
pub async fn release(docker: &Docker, image_ids: &[String]) -> Reclaimed {
    let mut out = Reclaimed::default();
    let mut seen = BTreeSet::new();
    for id in image_ids {
        if id.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        // Size before removal — afterwards there is nothing left to ask.
        let size = docker
            .inspect_image(id)
            .await
            .ok()
            .and_then(|i| i.size)
            .unwrap_or(0);
        match remove(docker, id).await {
            Ok(()) => {
                out.images += 1;
                out.bytes += size;
            }
            Err(e) => {
                // Almost always a 409: another container (often a sibling
                // preview on the same digest) still holds it.
                tracing::debug!(image = id, error = %format!("{e:#}"), "image not reclaimed");
            }
        }
    }
    if out.images > 0 {
        tracing::info!(%out, "reclaimed images on teardown");
    }
    out
}

/// The images to actually remove, oldest first, stopping once `need_bytes` is
/// covered. `need_bytes <= 0` reclaims nothing; `i64::MAX` takes everything
/// eligible.
///
/// Oldest-first because image age is the best available proxy for "least likely
/// to be wanted again" — Docker records no last-used time — so this keeps the
/// newest images, which are the plausible rollback targets, and gives up the
/// stale ones. Stopping at `need_bytes` means a node that is 2 GB over its
/// target gives up 2 GB of cache, not its whole cache.
///
/// Sizes are the per-image totals, which double-count shared layers exactly as
/// `docker system df` does, so the estimate is an upper bound on what is freed —
/// i.e. it may stop slightly early and re-fire on the next tick, rather than
/// over-reclaiming in one pass. That is the right way round.
pub fn plan(mut candidates: Vec<&ImageFacts>, need_bytes: i64) -> Vec<&ImageFacts> {
    candidates.sort_by_key(|i| (i.created, i.id.clone()));
    let mut freed = 0i64;
    let mut out = Vec::new();
    for img in candidates {
        if freed >= need_bytes {
            break;
        }
        freed += img.size;
        out.push(img);
    }
    out
}

/// Reclaim unused, unprotected images on one node until `need_bytes` is
/// recovered — the backstop path. Images only; see the module docs on why that
/// is structural.
pub async fn reclaim_unused(
    docker: &Docker,
    protected: &BTreeSet<String>,
    now: i64,
    min_age_secs: i64,
    need_bytes: i64,
) -> Result<Reclaimed> {
    // `all: false` lists final-layer images only. Their intermediate parents go
    // with them (`noprune: false` on the remove), which is where the bulk of the
    // reclaimable bytes actually live — 94 GB of the 105 GB on `private`.
    let summaries = docker
        .list_images(Some(qp::ListImagesOptions {
            all: false,
            ..Default::default()
        }))
        .await
        .context("listing images")?;
    let images: Vec<ImageFacts> = summaries
        .into_iter()
        .map(|i| ImageFacts {
            id: i.id,
            repo_tags: i.repo_tags,
            created: i.created,
            size: i.size,
        })
        .collect();

    // `all: true` — a *stopped* container still pins its image, and reclaiming
    // it would break the restart path.
    let containers = docker
        .list_containers(Some(qp::ListContainersOptions {
            all: true,
            ..Default::default()
        }))
        .await
        .context("listing containers")?;
    let in_use: BTreeSet<String> = containers.into_iter().filter_map(|c| c.image_id).collect();

    let candidates = plan(
        reclaimable(&images, &in_use, protected, now, min_age_secs),
        need_bytes,
    );
    let mut out = Reclaimed::default();
    for img in candidates {
        match remove(docker, &img.id).await {
            Ok(()) => {
                out.images += 1;
                out.bytes += img.size;
            }
            Err(e) => {
                tracing::debug!(image = img.id, error = %format!("{e:#}"), "image not reclaimed");
            }
        }
    }
    Ok(out)
}

/// One `remove_image`. Errors when Docker declines (the common, expected case).
///
/// `force: false` keeps the in-use interlock (the whole safety model here).
/// `noprune: false` lets the now-parentless layers go too — without it the
/// reclamation would delete the image record and leave the disk exactly as full.
///
/// Docker's delete response lists what it untagged and deleted, not how many
/// bytes that freed, so callers account the size themselves from the image
/// summary. Like `docker system df`, that sum double-counts shared layers and so
/// reads as an upper bound.
async fn remove(docker: &Docker, id: &str) -> Result<()> {
    docker
        .remove_image(
            id,
            Some(qp::RemoveImageOptions {
                force: false,
                noprune: false,
                ..Default::default()
            }),
            None,
        )
        .await?;
    Ok(())
}

/// Every image the platform itself runs, which reclamation must leave alone.
///
/// Collected from the modules that own them rather than re-listed here, so a
/// version bump in `platform`/`ingress` can't silently drop an image out of the
/// protected set.
pub fn protected_images(config: &crate::config::Config) -> BTreeSet<String> {
    let mut set: BTreeSet<String> = crate::platform::platform_images(&config.db_root_dir)
        .into_iter()
        .collect();
    set.extend(
        crate::ingress::ingress_images()
            .iter()
            .map(|s| s.to_string()),
    );
    set.insert(crate::secrets::HELPER_IMAGE.to_string());
    set.insert(config.term_helper_image.clone());
    set
}

/// The backstop loop: every `RECLAIM_INTERVAL`, reclaim images on any node over
/// the disk threshold.
///
/// Disk usage is read from the metrics snapshot the sampler already writes every
/// 15s rather than re-probed here — the probe costs a throwaway container per
/// node, and a backstop has no business adding load to a node that is already
/// out of disk.
pub async fn reclaim_loop(state: Arc<AppState>) {
    loop {
        tokio::time::sleep(RECLAIM_INTERVAL).await;
        if let Err(e) = tick(&state).await {
            tracing::warn!(error = %format!("{e:#}"), "image reclamation tick failed");
        }
    }
}

async fn tick(state: &AppState) -> Result<()> {
    if state.config.dry_run {
        return Ok(());
    }
    let disk_pct = cfg_f64(state, "reclaim_disk_pct", DEFAULT_DISK_PCT);
    let target_pct = cfg_f64(state, "reclaim_target_pct", DEFAULT_TARGET_PCT).min(disk_pct);
    let min_age_secs =
        cfg_f64(state, "reclaim_min_age_hours", DEFAULT_MIN_AGE_HOURS as f64) as i64 * 3_600;

    let Some((_, json)) = state.store.get_metrics_snapshot()? else {
        return Ok(()); // no sample yet (fresh boot) — next tick
    };
    let sampled: Vec<crate::metrics::NodeMetrics> = serde_json::from_str(&json)?;
    let pressured: Vec<&crate::metrics::NodeMetrics> = sampled
        .iter()
        .filter(|n| n.reachable && n.disk_total > 0)
        .filter(|n| n.disk_used as f64 / n.disk_total as f64 * 100.0 >= disk_pct)
        .collect();
    if pressured.is_empty() {
        return Ok(());
    }

    let platform = crate::snapshot::fetch(
        &state.http,
        &state.config,
        &state.config.root_org,
        "platform",
        "main",
    )
    .await?
    .context("platform snapshot unavailable")?;
    let nodes = NodesFile::parse(platform.files.get("nodes.yaml").context("no nodes.yaml")?)?;
    let protected = protected_images(&state.config);
    let now = unix_now();

    for sample in pressured {
        let Some(node) = nodes.nodes.iter().find(|n| n.name == sample.name) else {
            continue; // sampled but no longer in nodes.yaml
        };
        let used_pct = sample.disk_used as f64 / sample.disk_total as f64 * 100.0;
        tracing::warn!(
            node = node.name,
            used_pct = format!("{used_pct:.0}"),
            "node over the disk threshold — reclaiming unused images"
        );
        let docker = match state.nodes(&nodes).client_for(node).await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(node = node.name, error = %format!("{e:#}"), "reclaim: no Docker client");
                continue;
            }
        };
        // How much has to go for this node to reach the target, so a node a
        // little over its threshold gives up a little cache rather than all of it.
        let need_bytes = sample.disk_used - (sample.disk_total as f64 * target_pct / 100.0) as i64;
        match reclaim_unused(&docker, &protected, now, min_age_secs, need_bytes).await {
            Ok(out) if out.images > 0 => {
                tracing::info!(node = node.name, %out, needed_gb = format!("{:.1}", need_bytes as f64 / 1e9), "reclaimed unused images");
                // Recorded as an event so the disk that filled leaves a trail in
                // `majnet events`, rather than silently draining in the night.
                let _ = state.store.record(
                    "image-reclaim",
                    "",
                    &node.name,
                    "reclaim images",
                    &format!("disk {used_pct:.0}% — reclaimed {out}"),
                );
            }
            // Nothing eligible while over threshold means the disk is volumes,
            // running images, or images too new to touch — none of which
            // reclamation can fix. It is an operator problem, so say so loudly.
            Ok(_) => tracing::warn!(
                node = node.name,
                used_pct = format!("{used_pct:.0}"),
                "over the disk threshold with nothing reclaimable — needs an operator"
            ),
            Err(e) => {
                tracing::warn!(node = node.name, error = %format!("{e:#}"), "reclaim failed")
            }
        }
    }
    Ok(())
}

fn cfg_f64(state: &AppState, key: &str, default: f64) -> f64 {
    state
        .store
        .get_config(key)
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3_600;
    const NOW: i64 = 1_000 * HOUR;

    fn img(id: &str, tags: &[&str], age_hours: i64) -> ImageFacts {
        ImageFacts {
            id: id.to_string(),
            repo_tags: tags.iter().map(|s| s.to_string()).collect(),
            created: NOW - age_hours * HOUR,
            size: 1_000,
        }
    }

    fn ids(v: Vec<&ImageFacts>) -> Vec<&str> {
        v.iter().map(|i| i.id.as_str()).collect()
    }

    #[test]
    fn reclaims_only_old_unused_unprotected_images() {
        let images = [
            img("dead-preview", &["ghcr.io/o/app@sha256:1"], 200),
            img("running", &["ghcr.io/o/app@sha256:2"], 200),
            img("platform", &["postgres:17"], 200),
            img("fresh", &["ghcr.io/o/app@sha256:3"], 1),
        ];
        let in_use = BTreeSet::from(["running".to_string()]);
        let protected = BTreeSet::from(["postgres:17".to_string()]);

        let got = reclaimable(&images, &in_use, &protected, NOW, 72 * HOUR);
        assert_eq!(ids(got), ["dead-preview"]);
    }

    #[test]
    fn a_stopped_containers_image_is_in_use() {
        // Docker counts stopped containers too, and so must we — reclaiming
        // here would break `majnet deploy restart` on a stopped app.
        let images = [img("stopped-app", &["ghcr.io/o/app@sha256:1"], 500)];
        let in_use = BTreeSet::from(["stopped-app".to_string()]);
        assert!(reclaimable(&images, &in_use, &BTreeSet::new(), NOW, 72 * HOUR).is_empty());
    }

    #[test]
    fn an_untagged_image_is_still_reclaimable() {
        // Preview images are digest-pinned and frequently end up untagged; the
        // protected-tag rule must not accidentally shelter them.
        let images = [img("untagged", &[], 200)];
        let got = reclaimable(&images, &BTreeSet::new(), &BTreeSet::new(), NOW, 72 * HOUR);
        assert_eq!(ids(got), ["untagged"]);
    }

    #[test]
    fn min_age_protects_an_image_pulled_for_a_rollout_in_flight() {
        // Pulled, container not created yet — indistinguishable from garbage by
        // reference count alone, which is exactly what the age grace is for.
        let images = [img("just-pulled", &["ghcr.io/o/app@sha256:9"], 0)];
        assert!(
            reclaimable(&images, &BTreeSet::new(), &BTreeSet::new(), NOW, 72 * HOUR).is_empty()
        );
    }

    #[test]
    fn plan_takes_the_oldest_first_and_stops_once_it_has_enough() {
        // Newest images are the plausible rollback targets, so they go last.
        let images = [
            img("newest", &[], 10),
            img("oldest", &[], 300),
            img("middle", &[], 100),
        ];
        let all: Vec<&ImageFacts> = images.iter().collect();
        // Each is 1_000 bytes; needing 1_500 takes two.
        assert_eq!(ids(plan(all.clone(), 1_500)), ["oldest", "middle"]);
        // Needing nothing takes nothing, even with candidates available.
        assert!(plan(all.clone(), 0).is_empty());
        // Needing more than exists takes everything, oldest first.
        assert_eq!(ids(plan(all, i64::MAX)), ["oldest", "middle", "newest"]);
    }

    #[test]
    fn a_fast_churning_node_is_still_reclaimable() {
        // The shape measured on node `private`: 318 of 330 images built within
        // two days, disk at 96%. A 72 h age cut reclaimed 0 B there while the
        // node filled at ~4 GB/h. The safety floor is an anti-race guard, not a
        // retention window, so same-day images must still be reclaimable.
        let images: Vec<ImageFacts> = (0..300)
            .map(|k| img(&format!("preview-{k}"), &[], 6))
            .collect();
        let candidates = reclaimable(&images, &BTreeSet::new(), &BTreeSet::new(), NOW, HOUR);
        assert_eq!(candidates.len(), 300, "6 h-old images must be reclaimable");
        // And only as many as the shortfall needs.
        assert_eq!(plan(candidates, 2_500).len(), 3);
    }

    #[test]
    fn the_safety_floor_still_protects_an_in_flight_pull() {
        // The one thing age must protect: pulled, container not created yet.
        let images = [img("just-pulled", &[], 0)];
        assert!(reclaimable(&images, &BTreeSet::new(), &BTreeSet::new(), NOW, HOUR).is_empty());
    }

    #[test]
    fn one_protected_tag_shelters_the_whole_image() {
        // A platform image can carry several tags; any one of them protects it.
        let images = [img(
            "multi",
            &["ghcr.io/o/app@sha256:1", "busybox:stable"],
            900,
        )];
        let protected = BTreeSet::from(["busybox:stable".to_string()]);
        assert!(reclaimable(&images, &BTreeSet::new(), &protected, NOW, 72 * HOUR).is_empty());
    }
}
