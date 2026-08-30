//! Deploys.
//!
//! Almost nothing here deploys anything directly, and that is the design, not a
//! limitation: every state change is a commit on an `ops` repo, and the
//! reconciler converges from git. So `promote` writes the production overlay and
//! a render PR follows; `merge` merges that PR; `rollback` reverts ops `main`.
//! The output of each says what was *written*, because that is what happened —
//! the deploy is a consequence, and `majnet deploy progress` is where you watch
//! it land.
//!
//! `restart` is the one exception the design allows (§16): a restart changes no
//! state git owns, so it cannot be a commit. It is imperative, role-gated and
//! audited.

use anyhow::Result;
use clap::Subcommand;
use serde_json::Value;

use crate::client::{seg, Api};
use crate::output::{cell, elide, emit, rows, Table};
use crate::resolve;
use crate::App;

#[derive(Subcommand)]
pub enum DeployCmd {
    /// Copy the stable digest into the production overlay (a gated render PR
    /// follows; it does not deploy on its own).
    Promote {
        project: Option<String>,
        app: Option<String>,
    },
    /// Revert the ops repo's `main` head, which propagates as render PRs.
    Rollback { project: Option<String> },
    /// Restart an app's containers. Imperative, audited; changes nothing in git.
    Restart {
        project: Option<String>,
        app: Option<String>,
        #[arg(short, long)]
        class: Option<String>,
    },
    /// Render PRs waiting on an env branch.
    List { project: Option<String> },
    /// Merge a render PR — this is what actually ships it.
    ///
    /// The PR number comes first so the project can fall back to the context
    /// default: `majnet deploy merge 42`.
    Merge {
        /// PR number from `majnet deploy list`.
        #[arg(value_name = "PR")]
        number: u64,
        project: Option<String>,
    },
    /// Close a render PR without merging.
    Close {
        #[arg(value_name = "PR")]
        number: u64,
        project: Option<String>,
    },
    /// Rollouts currently in flight, per app and environment.
    Progress,
}

pub async fn run(app: &App, cmd: DeployCmd) -> Result<()> {
    match cmd {
        DeployCmd::Promote { project, app: name } => promote(app, project, name).await,
        DeployCmd::Rollback { project } => rollback(app, project).await,
        DeployCmd::Restart {
            project,
            app: name,
            class,
        } => restart(app, project, name, class).await,
        DeployCmd::List { project } => list(app, project).await,
        DeployCmd::Merge { number, project } => pr_action(app, project, number, "merge").await,
        DeployCmd::Close { number, project } => pr_action(app, project, number, "close").await,
        DeployCmd::Progress => progress(app).await,
    }
}

async fn promote(app: &App, project: Option<String>, name: Option<String>) -> Result<()> {
    let org = resolve::org(&app.client, &app.ctx, project.as_deref()).await?;
    let name = resolve::app(&app.ctx, name.as_deref())?;
    // Promotion targets production by definition, so it always asks.
    resolve::confirm("promote", &format!("{org}/{name}"), "production", app.yes)?;

    let message = app
        .client
        .send_text(
            Api::Bot,
            reqwest::Method::POST,
            &format!("/api/promote/{}/{}", seg(&org), seg(&name)),
            None,
        )
        .await?;
    println!("{message}");
    println!(
        "\nThat wrote the production overlay. The render PR onto env/production still needs an \
         admin review — `majnet deploy list {org}` to see it."
    );
    Ok(())
}

async fn rollback(app: &App, project: Option<String>) -> Result<()> {
    let org = resolve::org(&app.client, &app.ctx, project.as_deref()).await?;
    resolve::confirm("roll back", &org, "production", app.yes)?;
    let message = app
        .client
        .send_text(
            Api::Bot,
            reqwest::Method::POST,
            &format!("/api/rollback/{}", seg(&org)),
            None,
        )
        .await?;
    println!("{message}");
    Ok(())
}

async fn restart(
    app: &App,
    project: Option<String>,
    name: Option<String>,
    class: Option<String>,
) -> Result<()> {
    let org = resolve::org(&app.client, &app.ctx, project.as_deref()).await?;
    let name = resolve::app(&app.ctx, name.as_deref())?;
    let class = resolve::class(&app.ctx, class.as_deref())?;
    resolve::confirm("restart", &format!("{org}/{name}"), &class, app.yes)?;

    let message = app
        .client
        .send_text(
            Api::Recon,
            reqwest::Method::POST,
            &format!("/api/restart/{}/{}/{}", seg(&org), seg(&class), seg(&name)),
            None,
        )
        .await?;
    println!("{message}");
    Ok(())
}

async fn list(app: &App, project: Option<String>) -> Result<()> {
    let org = resolve::org(&app.client, &app.ctx, project.as_deref()).await?;
    let value = app
        .client
        .get_value(Api::Bot, &format!("/api/deploys/{}", seg(&org)))
        .await?;
    emit(app.format, &value, |v| {
        let mut table = Table::new(&["pr", "class", "title", "mergeable", "files", "created"])
            .empty_note("(no render PRs open — everything rendered is merged)");
        for pr in rows(v) {
            let files = pr
                .get("files")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            table.push(vec![
                cell(&pr, "number"),
                cell(&pr, "class"),
                cell(&pr, "title"),
                // `null` means GitHub is still computing mergeability — a real
                // third state, not a "no".
                match pr.get("mergeable") {
                    Some(Value::Bool(true)) => "yes".into(),
                    Some(Value::Bool(false)) => "NO".into(),
                    _ => "computing".into(),
                },
                files.to_string(),
                elide(&cell(&pr, "created_at"), 19),
            ]);
        }
        table.print();
        if table.len() > 0 {
            println!("\nMerge one: majnet deploy merge {org} <pr>");
        }
    })
}

async fn pr_action(app: &App, project: Option<String>, number: u64, action: &str) -> Result<()> {
    let org = resolve::org(&app.client, &app.ctx, project.as_deref()).await?;
    // A render PR onto env/production is the production gate; merging one is
    // the deploy. Ask, unless told not to.
    let class = pr_class(app, &org, number).await.unwrap_or_default();
    if action == "merge" {
        resolve::confirm("merge PR into", &format!("{org}#{number}"), &class, app.yes)?;
    }
    let message = app
        .client
        .send_text(
            Api::Bot,
            reqwest::Method::POST,
            &format!("/api/deploys/{}/{number}/{action}", seg(&org)),
            None,
        )
        .await?;
    println!("{message}");
    Ok(())
}

/// The env class a PR targets, so the confirmation prompt knows whether this is
/// production. Best-effort — a failure here must not block the action itself.
async fn pr_class(app: &App, org: &str, number: u64) -> Option<String> {
    let value = app
        .client
        .get_value(Api::Bot, &format!("/api/deploys/{}", seg(org)))
        .await
        .ok()?;
    rows(&value)
        .into_iter()
        .find(|pr| pr.get("number").and_then(Value::as_u64) == Some(number))
        .map(|pr| cell(&pr, "class"))
}

async fn progress(app: &App) -> Result<()> {
    let value = app
        .client
        .get_value(Api::Recon, "/api/deploy-progress")
        .await?;
    emit(app.format, &value, |v| {
        let mut table = Table::new(&["project", "app", "class", "stage", "status", "detail"])
            .empty_note("(nothing deploying right now)");
        for d in rows(v) {
            table.push(vec![
                cell(&d, "project"),
                cell(&d, "app"),
                cell(&d, "class"),
                cell(&d, "stage"),
                cell(&d, "status"),
                cell(&d, "detail"),
            ]);
        }
        table.print();
    })
}
