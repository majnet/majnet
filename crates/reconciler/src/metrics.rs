//! On-demand node + container metrics, gathered over the same per-node Docker
//! API (mTLS over WireGuard) the reconciler already uses to deploy — no
//! monitoring agents, no extra services. Surfaced read-only to the dashboard.

use anyhow::{Context, Result};
use futures_util::StreamExt;
use majnet_common::platform::{Node, NodesFile};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::AppState;

/// How often the sampler gathers + refreshes the latest snapshot. Short, so the
/// dashboard's running-state (served from the snapshot) is never very stale.
const SNAPSHOT_INTERVAL: Duration = Duration::from_secs(15);
/// Record a raw *history* point every Nth snapshot tick (≈60s — the raw-tier
/// resolution the RRD compaction bands assume).
const SAMPLE_EVERY: u64 = 4;
/// Run the compaction pass every Nth history sample (bands are day-scale — 15
/// min is plenty and keeps each pass cheap).
const COMPACT_EVERY: u64 = 15;

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Metrics sampler (ADR 0017): every 60s, gather node/host metrics and persist
/// one raw row per reachable node; periodically compact old rows into coarser
/// tiers. Independent of alerting — history is kept whether or not alerts are on.
pub async fn sample_loop(state: Arc<AppState>) {
    // Prime the snapshot immediately so `/api/metrics` is fast from the very
    // first request rather than waiting a full interval (or a live sweep).
    if let Err(e) = refresh_snapshot(&state).await {
        tracing::warn!(error = %format!("{e:#}"), "initial metrics snapshot failed");
    }
    let mut ticks: u64 = 0;
    let mut history_ticks: u64 = 0;
    loop {
        tokio::time::sleep(SNAPSHOT_INTERVAL).await;
        // One live gather per tick, reused for both the snapshot (every tick) and
        // the history sample (every SAMPLE_EVERY ticks) — no duplicate sweeps.
        let nodes = match gather(&state).await {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(error = %format!("{e:#}"), "metrics gather failed");
                continue;
            }
        };
        write_snapshot(&state, &nodes);
        ticks += 1;
        if !ticks.is_multiple_of(SAMPLE_EVERY) {
            continue;
        }
        write_history(&state, &nodes);
        history_ticks += 1;
        if history_ticks.is_multiple_of(COMPACT_EVERY) {
            let now = unix_now();
            if let Err(e) = state.store.compact_metrics(now) {
                tracing::warn!(error = %format!("{e:#}"), "metrics compaction failed");
            }
            if let Err(e) = state.store.compact_container_metrics(now) {
                tracing::warn!(error = %format!("{e:#}"), "container metrics compaction failed");
            }
        }
    }
}

/// Gather the fleet once and persist it as the latest snapshot (used to prime
/// the cache at startup so `/api/metrics` is fast from the first request).
async fn refresh_snapshot(state: &AppState) -> Result<()> {
    let nodes = gather(state).await?;
    write_snapshot(state, &nodes);
    Ok(())
}

/// Persist the whole fleet snapshot (running state, incl. per-container image +
/// state) as one JSON blob for `GET /api/metrics` to serve instantly.
fn write_snapshot(state: &AppState, nodes: &[NodeMetrics]) {
    match serde_json::to_string(nodes) {
        Ok(json) => {
            if let Err(e) = state.store.put_metrics_snapshot(unix_now(), &json) {
                tracing::warn!(error = %format!("{e:#}"), "metrics snapshot write failed");
            }
        }
        Err(e) => tracing::warn!(error = %format!("{e:#}"), "metrics snapshot serialize failed"),
    }
}

/// Record one raw history point per reachable node (node-level + per-container).
fn write_history(state: &AppState, nodes: &[NodeMetrics]) {
    let ts = unix_now();
    for n in nodes {
        if !n.reachable {
            continue; // don't record zero-rows for an unreachable node
        }
        if let Err(e) = state.store.insert_metric_sample(
            ts,
            &n.name,
            n.host_cpu_pct,
            n.mem_used,
            n.mem_total,
            n.containers_running,
            n.disk_used,
            n.disk_total,
        ) {
            tracing::warn!(error = %format!("{e:#}"), node = n.name, "metric sample write failed");
            continue;
        }
        for c in &n.apps {
            if let Err(e) = state.store.insert_container_sample(
                ts,
                &n.name,
                &c.name,
                c.cpu_pct,
                c.mem_used as i64,
                c.mem_limit as i64,
            ) {
                tracing::warn!(error = %format!("{e:#}"), container = c.name, "container sample write failed");
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct NodeMetrics {
    pub name: String,
    pub role: String,
    pub reachable: bool,
    pub error: Option<String>,
    pub cpus: i64,
    pub host_cpu_pct: f64,
    pub mem_total: i64,
    pub mem_used: i64,
    /// Bytes of image layers (`docker df`). Useful for *why* a disk is full;
    /// useless for *whether* it is, which is what `disk_total`/`disk_used` are
    /// for — this node reported 105 GB of images while nothing in `majnet
    /// status` could say the filesystem underneath had 0 bytes left.
    pub disk_images: i64,
    /// Filesystem backing Docker's data root: size and used, in bytes.
    pub disk_total: i64,
    pub disk_used: i64,
    pub containers: i64,
    pub containers_running: i64,
    pub server_version: String,
    pub os: String,
    pub kernel: String,
    pub apps: Vec<ContainerMetric>,
}

impl NodeMetrics {
    /// Disk usage as a percentage, or 0.0 when the probe did not report a
    /// filesystem (unreachable node, or a probe that timed out).
    pub fn disk_pct(&self) -> f64 {
        if self.disk_total > 0 {
            self.disk_used as f64 / self.disk_total as f64 * 100.0
        } else {
            0.0
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct ContainerMetric {
    pub name: String,
    pub image: String,
    pub state: String,
    pub cpu_pct: f64,
    pub mem_used: u64,
    pub mem_limit: u64,
}

/// Metrics for every node in `nodes.yaml`. Each node is probed with a short
/// timeout so an unreachable node (e.g. the parked private one) is reported as
/// such rather than hanging the whole response.
pub async fn gather(state: &AppState) -> Result<Vec<NodeMetrics>> {
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

    let mut out = Vec::new();
    for node in &nodes.nodes {
        let mut m = NodeMetrics {
            name: node.name.clone(),
            role: node.role.clone(),
            reachable: false,
            error: None,
            cpus: 0,
            host_cpu_pct: 0.0,
            mem_total: 0,
            mem_used: 0,
            disk_images: 0,
            disk_total: 0,
            disk_used: 0,
            containers: 0,
            containers_running: 0,
            server_version: String::new(),
            os: String::new(),
            kernel: String::new(),
            apps: Vec::new(),
        };
        match tokio::time::timeout(
            Duration::from_secs(20),
            collect(state, &nodes, node, &mut m),
        )
        .await
        {
            Ok(Ok(())) => m.reachable = true,
            Ok(Err(e)) => m.error = Some(format!("{e:#}")),
            Err(_) => m.error = Some("timeout — node unreachable".into()),
        }
        out.push(m);
    }
    Ok(out)
}

async fn collect(
    state: &AppState,
    nodes: &NodesFile,
    node: &Node,
    m: &mut NodeMetrics,
) -> Result<()> {
    let docker = state.nodes(nodes).client_for(node).await?;

    // Read info/df via their JSON (stable Docker field names) to avoid brittle
    // bollard struct-field coupling.
    let info = serde_json::to_value(&docker.info().await?).unwrap_or_default();
    m.cpus = info["NCPU"].as_i64().unwrap_or(0);
    m.mem_total = info["MemTotal"].as_i64().unwrap_or(0);
    m.containers = info["Containers"].as_i64().unwrap_or(0);
    m.containers_running = info["ContainersRunning"].as_i64().unwrap_or(0);
    m.server_version = info["ServerVersion"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    m.os = info["OperatingSystem"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    m.kernel = info["KernelVersion"]
        .as_str()
        .unwrap_or_default()
        .to_string();

    // `df` walks every image layer/volume to sum sizes — slow on a node with a
    // lot of image history (measured ~17 s on the small main node). It's a
    // supplementary disk metric, so bound it: a slow df must never push
    // `collect()` past its timeout and falsely mark the node unreachable (info()
    // already proved Docker answers). On timeout `disk_images` stays 0.
    if let Ok(Ok(df)) = tokio::time::timeout(
        Duration::from_secs(6),
        docker.df(None::<bollard::query_parameters::DataUsageOptions>),
    )
    .await
    {
        let dv = serde_json::to_value(&df).unwrap_or_default();
        m.disk_images = dv["LayersSize"].as_i64().unwrap_or(0);
    }

    let list = docker
        .list_containers(Some(bollard::query_parameters::ListContainersOptions {
            all: false,
            ..Default::default()
        }))
        .await?;
    // Per-container stats concurrently — a one-shot `stats` read blocks ~1s each,
    // so serially over many containers would blow the node budget. Run the host
    // /proc probe alongside them.
    let apps_fut = futures_util::future::join_all(list.into_iter().map(|c| {
        let docker = docker.clone();
        async move {
            let mut cm = ContainerMetric {
                name: c
                    .names
                    .as_ref()
                    .and_then(|n| n.first())
                    .map(|s| s.trim_start_matches('/').to_string())
                    .unwrap_or_default(),
                image: c.image.clone().unwrap_or_default(),
                state: c
                    .state
                    .map(|s| format!("{s:?}").to_lowercase())
                    .unwrap_or_default(),
                cpu_pct: 0.0,
                mem_used: 0,
                mem_limit: 0,
            };
            if let Some(id) = &c.id {
                let mut s = docker.stats(
                    id,
                    Some(bollard::query_parameters::StatsOptions {
                        stream: false,
                        one_shot: false,
                    }),
                );
                if let Ok(Some(Ok(stat))) =
                    tokio::time::timeout(Duration::from_secs(4), s.next()).await
                {
                    if let Ok(v) = serde_json::to_value(&stat) {
                        cm.cpu_pct = cpu_percent(&v);
                        cm.mem_used = v["memory_stats"]["usage"].as_u64().unwrap_or(0);
                        cm.mem_limit = v["memory_stats"]["limit"].as_u64().unwrap_or(0);
                    }
                }
            }
            cm
        }
    }));
    let (apps, host) = tokio::join!(apps_fut, host_probe(&docker));
    m.apps = apps;
    if let Some(h) = host {
        m.host_cpu_pct = h.cpu_pct;
        m.mem_used = h.mem_used;
        if m.mem_total == 0 {
            m.mem_total = h.mem_total;
        }
        m.disk_total = h.disk_total;
        m.disk_used = h.disk_used;
    }
    Ok(())
}

/// What one host probe reports. Bytes throughout.
pub struct HostProbe {
    pub cpu_pct: f64,
    pub mem_total: i64,
    pub mem_used: i64,
    pub disk_total: i64,
    pub disk_used: i64,
}

/// Host CPU%, memory and disk, read from inside a throwaway busybox container.
/// On plain Docker (no lxcfs) `/proc/stat` and `/proc/meminfo` reflect the host,
/// so this needs no host shell, agent, or privileges — just the Docker API.
///
/// Disk comes from `df` on the container's own `/`. That is an overlay whose
/// backing store *is* the filesystem holding Docker's data root, so its size and
/// used figures are the ones that matter: this is the disk that images fill and
/// the disk that hit 100% on node `private`. It is also exactly what an operator
/// sees from `majnet exec <app> -- df -h /`, which is how the outage had to be
/// diagnosed before this existed.
///
/// Best-effort with a hard internal deadline: the node already answered
/// `info()`, so a slow host probe must NOT hang `collect()` (its outer timeout
/// would then falsely mark the node unreachable). On a small/loaded node the
/// probe can outlive that outer timeout, which cancels this future mid-flight
/// and skips the helper's removal below — so we also `auto_remove` the helper
/// and sweep any orphans up front, making the probe self-healing rather than
/// leaking a container every slow tick.
async fn host_probe(docker: &bollard::Docker) -> Option<HostProbe> {
    sweep_helpers(docker).await;
    tokio::time::timeout(Duration::from_secs(8), host_probe_inner(docker))
        .await
        .ok()
        .flatten()
}

/// Remove helper containers orphaned by a previous tick whose `host_probe` was
/// cancelled by the outer `collect()` timeout before it could remove them
/// (`auto_remove` covers started helpers; a `Created`-but-never-started one only
/// this sweep catches). Cheap and best-effort — filter by our label in-process.
async fn sweep_helpers(docker: &bollard::Docker) {
    use bollard::query_parameters as qp;
    let orphans = docker
        .list_containers(Some(qp::ListContainersOptions {
            all: true,
            ..Default::default()
        }))
        .await
        .unwrap_or_default();
    for c in orphans {
        let is_helper = c
            .labels
            .as_ref()
            .is_some_and(|l| l.contains_key("majnet.helper"));
        if is_helper {
            if let Some(id) = &c.id {
                let _ = docker
                    .remove_container(
                        id,
                        Some(qp::RemoveContainerOptions {
                            force: true,
                            ..Default::default()
                        }),
                    )
                    .await;
            }
        }
    }
}

async fn host_probe_inner(docker: &bollard::Docker) -> Option<HostProbe> {
    use bollard::query_parameters as qp;
    if docker
        .inspect_image(crate::secrets::HELPER_IMAGE)
        .await
        .is_err()
    {
        // Bounded: a hung registry pull must not consume the probe deadline.
        let _ = tokio::time::timeout(Duration::from_secs(6), async {
            docker
                .create_image(
                    Some(qp::CreateImageOptions {
                        from_image: Some(crate::secrets::HELPER_IMAGE.into()),
                        ..Default::default()
                    }),
                    None,
                    None,
                )
                .collect::<Vec<_>>()
                .await
        })
        .await;
    }
    // Three `---`-separated sections: meminfo, two /proc/stat samples a second
    // apart, then the filesystem. `df -k` (not `-h`) so the numbers parse
    // exactly, and `/` because that overlay is backed by Docker's data-root
    // filesystem.
    let script = "grep -E '^MemTotal|^MemAvailable' /proc/meminfo; echo ---; \
                  grep '^cpu ' /proc/stat; sleep 1; grep '^cpu ' /proc/stat; echo ---; \
                  df -k /";
    let helper = docker
        .create_container(
            None::<qp::CreateContainerOptions>,
            bollard::models::ContainerCreateBody {
                image: Some(crate::secrets::HELPER_IMAGE.into()),
                cmd: Some(vec!["sh".into(), "-c".into(), script.into()]),
                labels: Some([("majnet.helper".to_string(), "metrics".to_string())].into()),
                // Docker removes the container on exit even if the explicit
                // remove below is skipped (outer timeout cancels this future).
                host_config: Some(bollard::models::HostConfig {
                    auto_remove: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .ok()?;

    let out = async {
        docker
            .start_container(&helper.id, None::<qp::StartContainerOptions>)
            .await?;
        // follow:true streams until the (short-lived) container exits.
        let mut logs = docker.logs(
            &helper.id,
            Some(qp::LogsOptions {
                stdout: true,
                stderr: false,
                follow: true,
                ..Default::default()
            }),
        );
        let mut buf = String::new();
        while let Some(Ok(chunk)) = logs.next().await {
            buf.push_str(&chunk.to_string());
        }
        Ok::<_, anyhow::Error>(buf)
    }
    .await;

    // Redundant with auto_remove on the happy path, but removes it promptly
    // instead of waiting on the exit event. Ignored if already gone.
    let _ = docker
        .remove_container(
            &helper.id,
            Some(qp::RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await;

    parse_proc(&out.ok()?)
}

fn parse_proc(s: &str) -> Option<HostProbe> {
    let mut sections = s.splitn(3, "---");
    let mem = sections.next()?;
    let cpu = sections.next()?;
    // The `df` section is the one part that may legitimately be missing (an
    // older probe payload, a busybox without `df`). Disk then reports 0, which
    // every consumer already reads as "not measured" rather than "empty disk".
    let disk = sections.next().unwrap_or("");
    let kb = |key: &str| -> Option<i64> {
        mem.lines()
            .find(|l| l.starts_with(key))?
            .split_whitespace()
            .nth(1)?
            .parse::<i64>()
            .ok()
            .map(|v| v * 1024)
    };
    let mem_total = kb("MemTotal")?;
    let mem_used = mem_total - kb("MemAvailable").unwrap_or(0);

    // Two `cpu ...` samples → busy fraction over the interval.
    let sample = |line: &str| -> Option<(f64, f64)> {
        let n: Vec<f64> = line
            .split_whitespace()
            .skip(1)
            .filter_map(|x| x.parse::<f64>().ok())
            .collect();
        if n.len() < 5 {
            return None;
        }
        let idle = n[3] + n[4]; // idle + iowait
        Some((n.iter().sum(), idle))
    };
    let mut cpu_lines = cpu.lines().filter(|l| l.starts_with("cpu "));
    let (t1, i1) = sample(cpu_lines.next()?)?;
    let (t2, i2) = sample(cpu_lines.next()?)?;
    let td = t2 - t1;
    let cpu_pct = if td > 0.0 {
        (((1.0 - (i2 - i1) / td) * 100.0 * 100.0).round() / 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    let (disk_total, disk_used) = parse_df(disk).unwrap_or((0, 0));
    Some(HostProbe {
        cpu_pct,
        mem_total,
        mem_used,
        disk_total,
        disk_used,
    })
}

/// `df -k /` output → (total_bytes, used_bytes) for the root filesystem.
///
/// Counted from the *right* — `… 1K-blocks Used Available Use% Mounted-on` —
/// and anchored on a `/` mount point. Counting from the left breaks on the two
/// shapes df actually produces: a device name long enough to wrap pushes the
/// numbers onto their own line (so `Used` lands where `1K-blocks` was), and a
/// device name containing a space shifts every column. Anchoring on the mount
/// point also skips the header, whose last field is `on`.
fn parse_df(s: &str) -> Option<(i64, i64)> {
    s.lines().find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        let n = f.len();
        if n < 5 || f[n - 1] != "/" {
            return None;
        }
        let total = f[n - 5].parse::<i64>().ok()?;
        let used = f[n - 4].parse::<i64>().ok()?;
        Some((total * 1024, used * 1024))
    })
}

/// Docker's container CPU% — the same formula `docker stats` uses, read from the
/// stats JSON (stable field names) to avoid brittle nested struct access.
fn cpu_percent(v: &serde_json::Value) -> f64 {
    let cur = v["cpu_stats"]["cpu_usage"]["total_usage"]
        .as_f64()
        .unwrap_or(0.0);
    let pre = v["precpu_stats"]["cpu_usage"]["total_usage"]
        .as_f64()
        .unwrap_or(0.0);
    let sys_cur = v["cpu_stats"]["system_cpu_usage"].as_f64().unwrap_or(0.0);
    let sys_pre = v["precpu_stats"]["system_cpu_usage"]
        .as_f64()
        .unwrap_or(0.0);
    let online = v["cpu_stats"]["online_cpus"]
        .as_f64()
        .unwrap_or(1.0)
        .max(1.0);
    let cpu_delta = cur - pre;
    let sys_delta = sys_cur - sys_pre;
    if sys_delta > 0.0 && cpu_delta > 0.0 {
        ((cpu_delta / sys_delta) * online * 100.0 * 100.0).round() / 100.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_df, parse_proc};

    const DF: &str = "Filesystem           1K-blocks      Used Available Use% Mounted on\n\
                      overlay              152672636 149000000   1000000  99% /\n";

    #[test]
    fn parses_meminfo_and_cpu_delta() {
        // idle goes 100→160 (Δ60) out of total 200→300 (Δ100) → 40% busy.
        let s = "MemTotal:       1000 kB\nMemAvailable:    400 kB\n---\n\
                 cpu  50 0 50 100 0 0 0 0\ncpu  90 0 50 160 0 0 0 0\n";
        let h = parse_proc(s).unwrap();
        assert_eq!(h.mem_total, 1000 * 1024);
        assert_eq!(h.mem_used, 600 * 1024);
        assert!((h.cpu_pct - 40.0).abs() < 0.01, "cpu={}", h.cpu_pct);
    }

    #[test]
    fn parses_disk_when_the_probe_reports_it() {
        let s = format!(
            "MemTotal:       1000 kB\nMemAvailable:    400 kB\n---\n\
             cpu  50 0 50 100 0 0 0 0\ncpu  90 0 50 160 0 0 0 0\n---\n{DF}"
        );
        let h = parse_proc(&s).unwrap();
        assert_eq!(h.disk_total, 152_672_636 * 1024);
        assert_eq!(h.disk_used, 149_000_000 * 1024);
    }

    #[test]
    fn a_probe_without_a_df_section_still_reports_cpu_and_memory() {
        // Disk must degrade on its own — an older payload or a busybox without
        // `df` cannot be allowed to cost the node its CPU/memory reporting.
        let s = "MemTotal:       1000 kB\nMemAvailable:    400 kB\n---\n\
                 cpu  50 0 50 100 0 0 0 0\ncpu  90 0 50 160 0 0 0 0\n";
        let h = parse_proc(s).unwrap();
        assert_eq!((h.disk_total, h.disk_used), (0, 0));
        assert_eq!(h.mem_total, 1000 * 1024);
    }

    #[test]
    fn df_skips_the_header_row() {
        assert_eq!(parse_df(DF), Some((152_672_636 * 1024, 149_000_000 * 1024)));
    }

    #[test]
    fn df_handles_a_wrapped_device_name() {
        // Long device names wrap onto their own line; that line carries no
        // numbers, so the numeric parse skips it rather than misreading it.
        let wrapped = "Filesystem 1K-blocks Used Available Use% Mounted on\n\
                       /dev/mapper/a-very-long-volume-group-name-here\n\
                       \u{20}         100 40 60 40% /\n";
        assert_eq!(parse_df(wrapped), Some((100 * 1024, 40 * 1024)));
    }
}
