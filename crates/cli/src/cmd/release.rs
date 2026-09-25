//! Releases (ADR 0009, ADR 0020).
//!
//! Nothing auto-releases. On every push to an app repo's `main` the bot
//! prepares a **draft** — the version it would cut and a changelog generated
//! from the conventional commits since the last tag — and then waits. Cutting
//! is an operator's act: `majnet release draft submit`, or `majnet release cut`
//! to skip the draft and tag straight away.
//!
//! `--bump auto` is the interesting default: it reads the commit messages
//! rather than asking you to remember whether anything was breaking.

use anyhow::{bail, Result};
use clap::{Args, Subcommand};
use serde_json::{json, Value};

use crate::client::{seg, Api};
use crate::output::{cell, elide, emit, rows, Table};
use crate::resolve;
use crate::App;

#[derive(Args, Clone)]
pub struct AppRef {
    pub project: Option<String>,
    pub app: Option<String>,
}

#[derive(Subcommand)]
pub enum ReleaseCmd {
    /// Releases already cut for an app.
    List(AppRef),
    /// Every app across the fleet with a draft waiting.
    Drafts,
    /// Tag a new version now, without going through the draft.
    Cut {
        #[command(flatten)]
        app: AppRef,
        /// Which part to increment; `auto` derives it from the commit messages.
        #[arg(long, default_value = "auto", value_parser = ["auto", "patch", "minor", "major"])]
        bump: String,
    },
    /// The pending draft for an app: version, bump, changelog.
    #[command(subcommand)]
    Draft(DraftCmd),
    /// Move an already-cut version into the stable track.
    ///
    /// The version comes first so project and app can fall back to the context
    /// defaults: `majnet release promote v1.4.0`.
    Promote {
        #[arg(value_name = "VERSION")]
        version: String,
        #[command(flatten)]
        app: AppRef,
    },
    /// Live progress of releases moving through the pipeline.
    Progress { project: Option<String> },
}

#[derive(Subcommand)]
pub enum DraftCmd {
    /// Show the pending draft.
    Show(AppRef),
    /// Recompute it from the current commits.
    Refresh(AppRef),
    /// Replace the changelog text.
    Notes {
        /// New notes; `-` reads them from stdin.
        #[arg(value_name = "NOTES")]
        notes: String,
        #[command(flatten)]
        app: AppRef,
    },
    /// Cut the draft: tag the repo and start the build.
    Submit(AppRef),
    /// Throw the draft away (the bot will prepare another on the next push).
    Discard(AppRef),
}

pub async fn run(app: &App, cmd: ReleaseCmd) -> Result<()> {
    match cmd {
        ReleaseCmd::List(r) => list(app, &r).await,
        ReleaseCmd::Drafts => drafts(app).await,
        ReleaseCmd::Cut { app: r, bump } => cut(app, &r, &bump).await,
        ReleaseCmd::Draft(sub) => draft(app, sub).await,
        ReleaseCmd::Promote { version, app: r } => promote(app, &r, &version).await,
        ReleaseCmd::Progress { project } => progress(app, project).await,
    }
}

/// (org, app) for a release command.
async fn refs(app: &App, r: &AppRef) -> Result<(String, String)> {
    let org = resolve::org(&app.client, &app.ctx, r.project.as_deref()).await?;
    let name = resolve::app(&app.ctx, r.app.as_deref())?;
    Ok((org, name))
}

async fn list(app: &App, r: &AppRef) -> Result<()> {
    let (org, name) = refs(app, r).await?;
    let value = app
        .client
        .get_value(
            Api::Bot,
            &format!("/api/releases/{}/{}", seg(&org), seg(&name)),
        )
        .await?;
    emit(app.format, &value, |v| {
        let mut table = Table::new(&["version", "commit", "published", "image"])
            .empty_note("(no releases cut yet)");
        for release in rows(v) {
            table.push(vec![
                cell(&release, "version"),
                elide(&cell(&release, "commit"), 8),
                elide(&cell(&release, "published_at"), 19),
                cell(&release, "app_image"),
            ]);
        }
        table.print();
    })
}

async fn drafts(app: &App) -> Result<()> {
    let value = app
        .client
        .get_value(Api::Bot, "/api/releases/drafts")
        .await?;
    emit(app.format, &value, |v| {
        let mut table = Table::new(&[
            "org", "app", "repo", "version", "bump", "commits", "updated",
        ])
        .empty_note("(nothing waiting to be released)");
        for d in rows(v) {
            table.push(vec![
                cell(&d, "org"),
                cell(&d, "app"),
                cell(&d, "repo"),
                cell(&d, "version"),
                cell(&d, "bump"),
                cell(&d, "commit_count"),
                elide(&cell(&d, "updated_at"), 19),
            ]);
        }
        table.print();
    })
}

async fn cut(app: &App, r: &AppRef, bump: &str) -> Result<()> {
    let (org, name) = refs(app, r).await?;
    // A cut tags the repo and starts a build that ends up deployable — worth a
    // prompt even though no environment changes yet.
    resolve::confirm(
        "cut a release for",
        &format!("{org}/{name}"),
        "production",
        app.yes,
    )?;
    let message = app
        .client
        .send_text(
            Api::Bot,
            reqwest::Method::POST,
            &format!(
                "/api/releases/{}/{}/cut?bump={}",
                seg(&org),
                seg(&name),
                seg(bump)
            ),
            None,
        )
        .await?;
    println!("{message}");
    println!("\nWatch it build: majnet release progress {org}");
    Ok(())
}

async fn draft(app: &App, cmd: DraftCmd) -> Result<()> {
    match cmd {
        DraftCmd::Show(r) => {
            let (org, name) = refs(app, &r).await?;
            let value = app
                .client
                .get_value(
                    Api::Bot,
                    &format!("/api/releases/{}/{}/draft", seg(&org), seg(&name)),
                )
                .await?;
            emit(app.format, &value, |v| {
                if v.is_null() {
                    println!("no draft for {org}/{name} — push to main, or `majnet release draft refresh`");
                    return;
                }
                println!("repo      {}", cell(v, "repo"));
                println!(
                    "version   {}  ({} bump)",
                    cell(v, "version"),
                    cell(v, "bump")
                );
                println!("base      {}", cell(v, "base"));
                println!("commits   {}", cell(v, "commit_count"));
                println!("edited    {}", cell(v, "notes_edited"));
                println!("\n{}", cell(v, "notes").trim_end());
                println!("\nCut it: majnet release draft submit {org} {name}");
            })
        }
        DraftCmd::Refresh(r) => {
            let (org, name) = refs(app, &r).await?;
            post_draft(app, &org, &name, "refresh", None).await
        }
        DraftCmd::Submit(r) => {
            let (org, name) = refs(app, &r).await?;
            resolve::confirm(
                "submit the release draft for",
                &format!("{org}/{name}"),
                "production",
                app.yes,
            )?;
            post_draft(app, &org, &name, "submit", None).await
        }
        DraftCmd::Discard(r) => {
            let (org, name) = refs(app, &r).await?;
            let message = app
                .client
                .send_text(
                    Api::Bot,
                    reqwest::Method::DELETE,
                    &format!("/api/releases/{}/{}/draft", seg(&org), seg(&name)),
                    None,
                )
                .await?;
            println!("{message}");
            Ok(())
        }
        DraftCmd::Notes { notes, app: r } => {
            let (org, name) = refs(app, &r).await?;
            let text = if notes == "-" {
                use std::io::Read as _;
                let mut buffer = String::new();
                std::io::stdin().read_to_string(&mut buffer)?;
                buffer
            } else {
                notes
            };
            if text.trim().is_empty() {
                bail!("refusing to save empty release notes");
            }
            let message = app
                .client
                .send_text(
                    Api::Bot,
                    reqwest::Method::PUT,
                    &format!("/api/releases/{}/{}/draft/notes", seg(&org), seg(&name)),
                    Some(&json!({ "notes": text })),
                )
                .await?;
            println!("{message}");
            Ok(())
        }
    }
}

async fn post_draft(
    app: &App,
    org: &str,
    name: &str,
    action: &str,
    body: Option<&Value>,
) -> Result<()> {
    let message = app
        .client
        .send_text(
            Api::Bot,
            reqwest::Method::POST,
            &format!("/api/releases/{}/{}/draft/{action}", seg(org), seg(name)),
            body,
        )
        .await?;
    println!("{message}");
    Ok(())
}

async fn promote(app: &App, r: &AppRef, version: &str) -> Result<()> {
    let (org, name) = refs(app, r).await?;
    let message = app
        .client
        .send_text(
            Api::Bot,
            reqwest::Method::POST,
            &format!(
                "/api/releases/{}/{}/promote/{}",
                seg(&org),
                seg(&name),
                seg(version)
            ),
            None,
        )
        .await?;
    println!("{message}");
    Ok(())
}

async fn progress(app: &App, project: Option<String>) -> Result<()> {
    let org = resolve::org(&app.client, &app.ctx, project.as_deref()).await?;
    let value = app
        .client
        .get_value(Api::Bot, &format!("/api/releases/progress/{}", seg(&org)))
        .await?;

    // The bot's view of a release is git-shaped and stops there: it reaches
    // `tracked` — which `set_release_stage` maps to `done` — once `track_stable`
    // re-points `env/stable` at the new tag. That is a commit, not a container.
    //
    // During the 2026-09-22 disk outage a release read green the whole way
    // through while every pull on the node failed with `no space left on
    // device` and nothing moved. The reconciler knew — it recorded each attempt
    // as `failed` — but nothing joined the two views, and the green one was the
    // one being watched.
    //
    // So join them here. The CLI already talks to both APIs, which makes this
    // the one place the contradiction is visible without a new endpoint. It is
    // the stopgap from ADR 0031, not the fix: the dashboard and the bot's own
    // API still report git-green, because only a reconciler→bot signal can fix
    // it at the source.
    //
    // Degrades on its own — a reconciler that is down leaves the column blank
    // rather than failing the command, because "cannot confirm" and "failed"
    // are different answers and this must not conflate them.
    let fleet = app
        .client
        .get_value(Api::Recon, "/api/deploy-progress")
        .await;
    let fleet_rows = fleet.as_ref().ok().map(rows).unwrap_or_default();
    // `stable` is the class a release auto-tracks into (ADR 0009); production
    // moves by a separate, reviewed promote.
    let fleet_for = |name: &str| -> Option<Value> {
        fleet_rows
            .iter()
            .find(|d| cell(d, "app") == name && cell(d, "class") == "stable")
            .cloned()
    };

    let joined: Vec<Value> = rows(&value)
        .into_iter()
        .map(|mut p| {
            let f = fleet_for(&cell(&p, "app"));
            if let Some(obj) = p.as_object_mut() {
                obj.insert(
                    "fleet".into(),
                    match &f {
                        Some(d) => json!({
                            "class": "stable",
                            "status": cell(d, "status"),
                            "stage": cell(d, "stage"),
                            "detail": cell(d, "detail"),
                        }),
                        None => Value::Null,
                    },
                );
            }
            p
        })
        .collect();

    emit(app.format, &Value::Array(joined.clone()), |_| {
        let mut table = Table::new(&[
            "app",
            "version",
            "stage",
            "status",
            "fleet (stable)",
            "detail",
            "updated",
        ])
        .empty_note("(no releases in flight)");
        for p in &joined {
            table.push(vec![
                cell(p, "app"),
                cell(p, "version"),
                cell(p, "stage"),
                cell(p, "status"),
                fleet_cell(p),
                cell(p, "detail"),
                elide(&cell(p, "updated_at"), 19),
            ]);
        }
        table.print();
        if joined.iter().any(contradicted) {
            println!(
                "\n! a release reads done while the reconciler has its stable rollout failed —\n\
                 \x20 the manifest was committed, the containers did not move. `majnet deploy progress`"
            );
        }
    })
}

/// What the reconciler says about this app's `stable` rollout. Blank when it
/// could not be asked — never "ok", which is the mistake this whole join exists
/// to stop making.
fn fleet_cell(p: &Value) -> String {
    match p.get("fleet") {
        Some(Value::Null) | None => String::new(),
        Some(f) => {
            let status = cell(f, "status");
            let stage = cell(f, "stage");
            if status == "failed" {
                format!("FAILED: {stage}")
            } else {
                format!("{status} ({stage})")
            }
        }
    }
}

/// A release claiming success while the reconciler has that app's stable
/// rollout failed. The pair the outage produced, and the only combination worth
/// shouting about.
fn contradicted(p: &Value) -> bool {
    cell(p, "status") == "done"
        && p.get("fleet")
            .map(|f| cell(f, "status") == "failed")
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{contradicted, fleet_cell};
    use serde_json::json;

    fn row(status: &str, fleet: serde_json::Value) -> serde_json::Value {
        json!({ "app": "api", "status": status, "fleet": fleet })
    }

    #[test]
    fn a_done_release_over_a_failed_rollout_is_contradicted() {
        // The 2026-09-22 shape: env/stable committed, containers never moved.
        let p = row("done", json!({ "status": "failed", "stage": "pulling" }));
        assert!(contradicted(&p));
        assert_eq!(fleet_cell(&p), "FAILED: pulling");
    }

    #[test]
    fn agreement_is_not_contradiction() {
        let p = row("done", json!({ "status": "done", "stage": "deployed" }));
        assert!(!contradicted(&p));
        assert_eq!(fleet_cell(&p), "done (deployed)");
    }

    #[test]
    fn an_unreachable_reconciler_reads_blank_not_ok() {
        // "Cannot confirm" must never render as success — that conflation is
        // the whole bug this join exists to stop.
        let p = row("done", serde_json::Value::Null);
        assert!(!contradicted(&p));
        assert_eq!(fleet_cell(&p), "");
    }

    #[test]
    fn a_release_still_in_flight_is_not_contradicted() {
        // Only a release claiming success can contradict the fleet.
        let p = row("active", json!({ "status": "failed", "stage": "health" }));
        assert!(!contradicted(&p));
    }
}
