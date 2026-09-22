//! The read side: what exists, what is running, and what it is saying.
//!
//! Two backends answer these. The **bot** knows what git says — projects, apps,
//! manifests, members. The **reconciler** knows what is actually running —
//! containers, logs, metrics, events. `status` is the one command that joins
//! them, because "is the fleet all right" is a question neither can answer
//! alone.

use anyhow::{Context as _, Result};
use clap::Args;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::time::Duration;

use crate::client::{seg, Api};
use crate::output::{bytes, cell, elide, emit, rows, table_from, Table};
use crate::resolve;
use crate::App;

/// A project + app + class, the shape most commands need.
#[derive(Args, Clone)]
pub struct TargetArgs {
    /// Project name or GitHub org (default: the context's).
    pub project: Option<String>,
    /// App name (default: the context's).
    pub app: Option<String>,
    /// Environment class: production | stable | testing | ephemeral.
    #[arg(short, long)]
    pub class: Option<String>,
}

/// project, app, class — resolved once, so every command reports the same target.
pub async fn target(app: &App, args: &TargetArgs) -> Result<(String, String, String)> {
    let org = resolve::org(&app.client, &app.ctx, args.project.as_deref()).await?;
    let name = resolve::app(&app.ctx, args.app.as_deref())?;
    let class = resolve::class(&app.ctx, args.class.as_deref())?;
    Ok((org, name, class))
}

// ── status ───────────────────────────────────────────────────────────────────

/// The "what is going on right now" screen: who you are, whether every node is
/// reachable, what is mid-deploy, and anything that recently failed.
///
/// Each section degrades on its own. A reconciler that is down must not stop
/// the command from telling you the reconciler is down.
pub async fn status(app: &App) -> Result<()> {
    let who = super::auth::identity(app).await.ok();
    let projects = app.client.get_value(Api::Bot, "/api/projects").await;
    let metrics = app.client.get_value(Api::Recon, "/api/metrics").await;
    let progress = app
        .client
        .get_value(Api::Recon, "/api/deploy-progress")
        .await;
    let events = app
        .client
        .get_value(Api::Recon, "/api/events?limit=200")
        .await;

    if !app.table() {
        let value = json!({
            "identity": who.as_ref().map(|w| json!({"login": w.login, "platform_admin": w.admin})),
            "projects": projects.as_ref().ok(),
            "nodes": metrics.as_ref().ok(),
            // Active rollouts only — as the name says, and as the table below
            // has always rendered. This used to carry the whole
            // `deploy_progress` table, so a terminal row that nothing had
            // overwritten read as a live deploy: four apps showed `failed` here
            // at 27 days old while all four were serving. The reconciler now
            // retires a resolved failure when the app converges, but a reader of
            // this field should not depend on that to avoid seeing history.
            "deploys_in_flight": progress.as_ref().ok().map(|p| {
                Value::Array(rows(p).into_iter().filter(|d| cell(d, "status") == "active").collect())
            }),
            "recent_failures": events.as_ref().ok().map(|e| {
                Value::Array(rows(e).into_iter().filter(failed).take(20).collect())
            }),
            "errors": section_errors(&[
                ("projects", &projects), ("metrics", &metrics),
                ("deploy-progress", &progress), ("events", &events),
            ]),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }

    if let Some(w) = &who {
        println!("identity  {}", super::auth::identity_line(w));
    }
    if let Ok(p) = &projects {
        let list = rows(p);
        println!(
            "projects  {} registered · {} app(s)",
            list.len(),
            list.iter()
                .filter_map(|p| p.get("apps").and_then(Value::as_u64))
                .sum::<u64>()
        );
    }

    println!();
    match &metrics {
        Ok(m) => node_table(&rows(m)).print(),
        Err(e) => println!("nodes: unavailable — {e:#}"),
    }

    if let Ok(p) = &progress {
        let live: Vec<Value> = rows(p)
            .into_iter()
            .filter(|d| cell(d, "status") == "active")
            .collect();
        if !live.is_empty() {
            println!("\nin flight");
            let mut table = Table::new(&["project", "app", "class", "stage", "detail"]);
            for d in &live {
                table.push(vec![
                    cell(d, "project"),
                    cell(d, "app"),
                    cell(d, "class"),
                    cell(d, "stage"),
                    cell(d, "detail"),
                ]);
            }
            table.print();
        }
    }

    if let Ok(e) = &events {
        let failures: Vec<Value> = rows(e).into_iter().filter(failed).take(10).collect();
        println!();
        if failures.is_empty() {
            println!("no failures in the last 200 events");
        } else {
            println!("recent failures");
            event_table(&failures).print();
            println!("\n`majnet events --failed` for more");
        }
    }
    Ok(())
}

fn section_errors(sections: &[(&str, &Result<Value>)]) -> Value {
    let mut map = serde_json::Map::new();
    for (name, result) in sections {
        if let Err(e) = result {
            map.insert((*name).to_string(), json!(format!("{e:#}")));
        }
    }
    Value::Object(map)
}

// ── events ───────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct EventArgs {
    /// Show at most this many.
    #[arg(long, default_value_t = 40)]
    pub limit: usize,
    /// Only this project (matched on the event's project field).
    #[arg(long)]
    pub project: Option<String>,
    /// Only entries that look like failures.
    #[arg(long)]
    pub failed: bool,
    /// Keep printing new entries as they arrive.
    #[arg(short, long)]
    pub follow: bool,
    /// Read the bot's log instead of the reconciler's (git-side activity).
    #[arg(long)]
    pub bot: bool,
}

pub async fn events(app: &App, args: &EventArgs) -> Result<()> {
    let (api, path) = if args.bot {
        (Api::Bot, "/api/events".to_string())
    } else {
        (
            Api::Recon,
            format!("/api/events?limit={}", args.limit.max(200)),
        )
    };

    if !args.follow {
        let value = app.client.get_value(api, &path).await?;
        let selected = select_events(&value, args);
        return emit(app.format, &Value::Array(selected.clone()), |_| {
            let table = event_table(&selected);
            let count = table.len();
            table.print();
            if count > 0 {
                let failures = selected.iter().filter(|e| failed(e)).count();
                println!(
                    "\n{count} shown · {failures} look{} like a failure",
                    if failures == 1 { "s" } else { "" }
                );
            }
        });
    }

    // Follow: the API has no stream, so poll and print what we have not seen.
    // Keyed on the whole row rather than the timestamp — the reconciler writes
    // several events within one second and a timestamp cursor would drop them.
    let mut seen: VecDeque<String> = VecDeque::new();
    let mut first = true;
    loop {
        match app.client.get_value(api, &path).await {
            Ok(value) => {
                let mut fresh: Vec<Value> = Vec::new();
                for event in select_events(&value, args).into_iter().rev() {
                    let key = event.to_string();
                    if seen.contains(&key) {
                        continue;
                    }
                    if seen.len() > 500 {
                        seen.pop_front();
                    }
                    seen.push_back(key);
                    fresh.push(event);
                }
                if first {
                    // Print the backlog once so --follow isn't a blank screen.
                    event_table(&fresh.iter().rev().cloned().collect::<Vec<_>>()).print();
                    first = false;
                } else {
                    for event in &fresh {
                        println!(
                            "{}  {}  {}  {}",
                            elide(&cell(event, "at"), 19),
                            elide(&cell(event, "project"), 16),
                            elide(&cell(event, "action"), 28),
                            cell(event, "result")
                        );
                    }
                }
            }
            Err(e) => eprintln!("majnet: {e:#}"),
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

fn select_events(value: &Value, args: &EventArgs) -> Vec<Value> {
    rows(value)
        .into_iter()
        .filter(|e| {
            args.project
                .as_deref()
                .is_none_or(|p| cell(e, "project").eq_ignore_ascii_case(p))
        })
        .filter(|e| !args.failed || failed(e))
        .take(args.limit)
        .collect()
}

/// How the reconciler words a failure in `result` — the same heuristic the
/// dashboard's feed uses to colour a row red.
fn failed(event: &Value) -> bool {
    let result = cell(event, "result").to_ascii_lowercase();
    result.contains("failed") || result.contains("error") || result.contains("unhealthy")
}

fn event_table(events: &[Value]) -> Table {
    let mut table = Table::new(&["at", "project", "action", "commit", "result"])
        .empty_note("no matching events");
    for e in events {
        table.push(vec![
            elide(&cell(e, "at"), 19),
            cell(e, "project"),
            cell(e, "action"),
            elide(&cell(e, "commit"), 8),
            cell(e, "result"),
        ]);
    }
    table
}

// ── fleet ────────────────────────────────────────────────────────────────────

pub async fn nodes(app: &App) -> Result<()> {
    let value = app.client.get_value(Api::Bot, "/api/nodes").await?;
    emit(app.format, &value, |v| {
        table_from(v, &["name", "role", "wireguard_ip", "public_endpoint"]).print()
    })
}

pub async fn metrics(app: &App, node: Option<&str>) -> Result<()> {
    let value = app.client.get_value(Api::Recon, "/api/metrics").await?;
    let selected: Vec<Value> = rows(&value)
        .into_iter()
        .filter(|n| node.is_none_or(|want| cell(n, "name") == want))
        .collect();
    emit(app.format, &Value::Array(selected.clone()), |_| {
        node_table(&selected).print();
        // Per-container detail only makes sense for one node; for the whole
        // fleet it is a wall of rows nobody reads.
        if let Some(one) = selected.first().filter(|_| selected.len() == 1) {
            let apps = one.get("apps").cloned().unwrap_or(Value::Null);
            let containers = rows(&apps);
            if !containers.is_empty() {
                println!("\ncontainers on {}", cell(one, "name"));
                let mut table = Table::new(&["name", "state", "cpu%", "memory", "image"]);
                for c in &containers {
                    table.push(vec![
                        cell(c, "name"),
                        cell(c, "state"),
                        format!("{:.1}", number(c, "cpu_pct")),
                        format!(
                            "{} / {}",
                            bytes(number(c, "mem_used")),
                            bytes(number(c, "mem_limit"))
                        ),
                        cell(c, "image"),
                    ]);
                }
                table.print();
            }
        }
    })
}

fn node_table(nodes: &[Value]) -> Table {
    let mut table = Table::new(&[
        "node",
        "role",
        "state",
        "cpu%",
        "memory",
        "disk",
        "containers",
    ])
    .empty_note("(no nodes reporting)");
    for n in nodes {
        let reachable = n.get("reachable").and_then(Value::as_bool).unwrap_or(false);
        let state = if reachable {
            "ok".to_string()
        } else {
            // The error is the whole message when a node is down — don't hide
            // it behind a bare "unreachable".
            format!("UNREACHABLE: {}", cell(n, "error"))
        };
        let memory = if reachable {
            format!(
                "{} / {}",
                bytes(number(n, "mem_used")),
                bytes(number(n, "mem_total"))
            )
        } else {
            String::new()
        };
        table.push(vec![
            cell(n, "name"),
            cell(n, "role"),
            state,
            if reachable {
                format!("{:.0}", number(n, "host_cpu_pct"))
            } else {
                String::new()
            },
            memory,
            disk_cell(n, reachable),
            if reachable {
                format!(
                    "{}/{}",
                    cell(n, "containers_running"),
                    cell(n, "containers")
                )
            } else {
                String::new()
            },
        ]);
    }
    table
}

fn number(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

/// The node's disk cell: `used / total (NN%)`, flagged once it is worth acting
/// on. Nothing in `majnet` reported node disk before — a node filled to 100%,
/// took `stable` down with it, and finding that out needed a shell on the host.
///
/// `!` at the reclamation threshold (85%) and `!!` near full, because by the
/// time a disk is full the deploys that would fix it cannot pull.
fn disk_cell(n: &Value, reachable: bool) -> String {
    let total = number(n, "disk_total");
    if !reachable || total <= 0.0 {
        // An older reconciler, or a probe that timed out. Say nothing rather
        // than print a confident 0% for a disk nobody measured.
        return String::new();
    }
    let used = number(n, "disk_used");
    let pct = used / total * 100.0;
    let flag = match pct {
        p if p >= 95.0 => " !!",
        p if p >= 85.0 => " !",
        _ => "",
    };
    format!("{} / {} ({pct:.0}%){flag}", bytes(used), bytes(total))
}

// ── projects and apps ────────────────────────────────────────────────────────

pub async fn projects(app: &App) -> Result<()> {
    let value = app.client.get_value(Api::Bot, "/api/projects").await?;
    emit(app.format, &value, |v| {
        table_from(v, &["name", "org", "apps", "onboarded"]).print()
    })
}

pub async fn apps(app: &App, project: Option<&str>) -> Result<()> {
    let org = resolve::org(&app.client, &app.ctx, project).await?;
    let value = app
        .client
        .get_value(Api::Bot, &format!("/api/apps/{}", seg(&org)))
        .await?;
    emit(app.format, &value, |v| {
        table_from(v, &["name", "classes", "database", "host", "image"]).print()
    })
}

/// One app, from every angle the API can offer — the CLI's answer to opening
/// the dashboard's app page.
pub async fn app_detail(app: &App, project: Option<&str>, name: Option<&str>) -> Result<()> {
    let org = resolve::org(&app.client, &app.ctx, project).await?;
    let name = resolve::app(&app.ctx, name)?;

    let all = app
        .client
        .get_value(Api::Bot, &format!("/api/apps/{}", seg(&org)))
        .await?;
    let summary = rows(&all)
        .into_iter()
        .find(|a| cell(a, "name") == name)
        .with_context(|| format!("{org} has no app '{name}'"))?;

    let info = app
        .client
        .get_value(
            Api::Recon,
            &format!("/api/info/{}/{}", seg(&org), seg(&name)),
        )
        .await
        .unwrap_or(Value::Null);

    // Containers per declared class, so "what is actually up" sits next to
    // "what is declared" — the two disagreeing is the usual reason to look.
    let classes: Vec<String> = summary
        .get("classes")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|c| c.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut containers = serde_json::Map::new();
    for class in &classes {
        let path = format!(
            "/api/containers/{}/{}/{}",
            seg(&org),
            seg(class),
            seg(&name)
        );
        match app.client.get_value(Api::Recon, &path).await {
            Ok(v) => {
                containers.insert(class.clone(), v);
            }
            Err(e) => {
                containers.insert(class.clone(), json!({"error": format!("{e:#}")}));
            }
        }
    }

    let value = json!({
        "app": summary,
        "build_info": info,
        "containers": Value::Object(containers.clone()),
    });
    emit(app.format, &value, |_| {
        println!("{name}  ({org})");
        println!("image     {}", cell(&summary, "image"));
        println!("classes   {}", cell(&summary, "classes"));
        if !cell(&summary, "host").is_empty() {
            println!("host      {}", cell(&summary, "host"));
        }
        if !cell(&summary, "database").is_empty() {
            println!("database  {}", cell(&summary, "database"));
        }
        if !cell(&summary, "repo").is_empty() {
            println!("repo      {}", cell(&summary, "repo"));
        }

        let builds = rows(&info);
        if !builds.is_empty() {
            println!("\nbuild info");
            let mut table = Table::new(&["class", "commit", "at", "reported"]);
            for b in &builds {
                let reported = b
                    .get("info")
                    .filter(|i| !i.is_null())
                    .map(|i| i.to_string())
                    .unwrap_or_else(|| cell(b, "error"));
                table.push(vec![
                    cell(b, "class"),
                    elide(&cell(b, "commit"), 8),
                    elide(&cell(b, "at"), 19),
                    reported,
                ]);
            }
            table.print();
        }

        for (class, value) in &containers {
            println!("\ncontainers · {class}");
            if let Some(err) = value.get("error") {
                println!("  unavailable — {}", err.as_str().unwrap_or_default());
                continue;
            }
            container_table(value).print();
        }
    })
}

pub async fn ps(app: &App, args: &TargetArgs) -> Result<()> {
    let (org, name, class) = target(app, args).await?;
    let value = app
        .client
        .get_value(
            Api::Recon,
            &format!(
                "/api/containers/{}/{}/{}",
                seg(&org),
                seg(&class),
                seg(&name)
            ),
        )
        .await?;
    emit(app.format, &value, |v| container_table(v).print())
}

fn container_table(value: &Value) -> Table {
    let mut table = Table::new(&["name", "state", "status", "image"])
        .empty_note("(nothing running for this app in this environment)");
    for c in rows(value) {
        table.push(vec![
            cell(&c, "name"),
            cell(&c, "state"),
            cell(&c, "status"),
            cell(&c, "image"),
        ]);
    }
    table
}

// ── logs ─────────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct LogArgs {
    #[command(flatten)]
    pub target: TargetArgs,
    /// Lines to fetch.
    #[arg(short = 'n', long, default_value_t = 300)]
    pub tail: usize,
    /// Keep printing as new lines arrive (polls; the API has no stream).
    #[arg(short, long)]
    pub follow: bool,
}

pub async fn logs(app: &App, args: &LogArgs) -> Result<()> {
    let (org, name, class) = target(app, &args.target).await?;
    let path = format!(
        "/api/logs/{}/{}/{}?tail={}",
        seg(&org),
        seg(&class),
        seg(&name),
        args.tail
    );

    if !args.follow {
        let text = app.client.get_text(Api::Recon, &path).await?;
        print!("{text}");
        if !text.ends_with('\n') {
            println!();
        }
        return Ok(());
    }

    // Poll and print the tail we have not printed. Docker's tail is a moving
    // window, so anchor on the last line we showed and print what follows it;
    // if that line has already scrolled out, print the whole window rather than
    // silently skipping the gap.
    eprintln!("following {org}/{name} ({class}) — ctrl-c to stop");
    let mut anchor: Option<String> = None;
    loop {
        match app.client.get_text(Api::Recon, &path).await {
            Ok(text) => {
                let lines: Vec<&str> = text.lines().collect();
                let start = match &anchor {
                    None => 0,
                    Some(last) => match lines.iter().rposition(|l| l == last) {
                        Some(i) => i + 1,
                        None => {
                            eprintln!("majnet: log window advanced past the last line shown");
                            0
                        }
                    },
                };
                for line in &lines[start.min(lines.len())..] {
                    println!("{line}");
                }
                if let Some(last) = lines.last() {
                    anchor = Some((*last).to_string());
                }
            }
            Err(e) => eprintln!("majnet: {e:#}"),
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

// ── manifests, members, secrets, info, db ────────────────────────────────────

pub async fn info(app: &App, project: Option<&str>, name: Option<&str>) -> Result<()> {
    let org = resolve::org(&app.client, &app.ctx, project).await?;
    let name = resolve::app(&app.ctx, name)?;
    let value = app
        .client
        .get_value(
            Api::Recon,
            &format!("/api/info/{}/{}", seg(&org), seg(&name)),
        )
        .await?;
    emit(app.format, &value, |v| {
        let mut table = Table::new(&["class", "commit", "at", "reported"])
            .empty_note("(no /info scraped yet for this app)");
        for b in rows(v) {
            let reported = b
                .get("info")
                .filter(|i| !i.is_null())
                .map(|i| i.to_string())
                .unwrap_or_else(|| cell(&b, "error"));
            table.push(vec![
                cell(&b, "class"),
                elide(&cell(&b, "commit"), 8),
                elide(&cell(&b, "at"), 19),
                reported,
            ]);
        }
        table.print();
    })
}

pub async fn manifest(app: &App, project: Option<&str>, name: Option<&str>) -> Result<()> {
    let org = resolve::org(&app.client, &app.ctx, project).await?;
    let name = resolve::app(&app.ctx, name)?;
    let value = app
        .client
        .get_value(
            Api::Bot,
            &format!("/api/manifest/{}/{}", seg(&org), seg(&name)),
        )
        .await?;
    emit(app.format, &value, |v| {
        // The API returns { "base.yaml": {yaml, data}, … }; the YAML is the
        // thing a human came for, so print it as the files it is.
        if let Some(files) = v.as_object() {
            for (file, content) in files {
                println!("── {file} ──");
                match content.get("yaml").and_then(Value::as_str) {
                    Some(yaml) => println!("{}", yaml.trim_end()),
                    None => println!("{content}"),
                }
                println!();
            }
        } else {
            println!("{v}");
        }
    })
}

pub async fn members(app: &App, project: Option<&str>) -> Result<()> {
    let org = resolve::org(&app.client, &app.ctx, project).await?;
    let value = app
        .client
        .get_value(Api::Bot, &format!("/api/members/{}", seg(&org)))
        .await?;
    emit(app.format, &value, |v| {
        table_from(v, &["user", "role"]).print()
    })
}

#[derive(Args)]
pub struct SecretArgs {
    #[command(flatten)]
    pub target: TargetArgs,
    /// Print the decrypted values, not just the names.
    #[arg(long)]
    pub reveal: bool,
}

/// Secret **names** by default.
///
/// The values are available — the reconciler decrypts them for the dashboard's
/// editor under the same role gate — but printing them because someone typed
/// `majnet secrets` is how a password ends up in a shell history or a CI log.
/// `--reveal` is the deliberate act.
pub async fn secrets(app: &App, args: &SecretArgs) -> Result<()> {
    let (org, name, class) = target(app, &args.target).await?;
    let value = app
        .client
        .get_value(
            Api::Recon,
            &format!("/api/secrets/{}/{}/{}", seg(&org), seg(&class), seg(&name)),
        )
        .await?;

    let masked = match (&value, args.reveal) {
        (Value::Object(map), false) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let hint = v.as_str().map_or(0, str::len);
                    (
                        k.clone(),
                        json!(format!("<{hint} chars — --reveal to show>")),
                    )
                })
                .collect(),
        ),
        _ => value.clone(),
    };
    emit(app.format, &masked, |v| {
        let mut table =
            Table::new(&["name", "value"]).empty_note("(no secrets for this environment)");
        if let Some(map) = v.as_object() {
            for (k, val) in map {
                table.push(vec![
                    k.clone(),
                    val.as_str().unwrap_or_default().to_string(),
                ]);
            }
        }
        table.print();
    })
}

pub async fn db(app: &App, args: &TargetArgs) -> Result<()> {
    let (org, name, class) = target(app, args).await?;
    let value = app
        .client
        .get_value(
            Api::Recon,
            &format!("/api/db/{}/{}/{}", seg(&org), seg(&class), seg(&name)),
        )
        .await?;
    emit(app.format, &value, |v| {
        println!("engine    {}", cell(v, "engine"));
        println!("database  {}", cell(v, "database"));
        println!("user      {}", cell(v, "user"));
        println!("container {}", cell(v, "container"));
        println!("node      {}", cell(v, "node"));
        println!("\nQuery it: majnet sql {org} {name} --class {class} 'SELECT 1'");
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(project: &str, result: &str) -> Value {
        json!({"at": "2026-08-30 12:00:00", "project": project, "action": "converge",
               "commit": "abc123def", "result": result})
    }

    #[test]
    fn failure_detection_matches_how_the_reconciler_words_it() {
        assert!(failed(&event("demo", "FAILED: boom")));
        assert!(failed(&event("demo", "container reported unhealthy")));
        assert!(failed(&event("demo", "render error")));
        assert!(!failed(&event("demo", "deployed abc123")));
    }

    #[test]
    fn event_selection_filters_by_project_and_failure_and_respects_the_limit() {
        let all = json!([
            event("demo", "FAILED: boom"),
            event("demo", "deployed"),
            event("other", "FAILED: nope"),
        ]);
        let args = EventArgs {
            limit: 10,
            project: Some("DEMO".into()),
            failed: true,
            follow: false,
            bot: false,
        };
        let picked = select_events(&all, &args);
        assert_eq!(picked.len(), 1);
        assert_eq!(cell(&picked[0], "result"), "FAILED: boom");

        let capped = EventArgs {
            limit: 1,
            project: None,
            failed: false,
            follow: false,
            bot: false,
        };
        assert_eq!(select_events(&all, &capped).len(), 1);
    }

    /// An unreachable node must carry its error into the table, not vanish
    /// behind a blank row.
    #[test]
    fn an_unreachable_node_keeps_its_error() {
        let nodes = vec![json!({"name": "prod", "role": "prod", "reachable": false,
                                "error": "connection refused"})];
        assert_eq!(node_table(&nodes).len(), 1);
    }
}
