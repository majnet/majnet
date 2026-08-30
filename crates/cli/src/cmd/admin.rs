//! Platform-admin commands: what the control plane itself is running.
//!
//! The control plane updates the same way apps do — its version is pinned in
//! the platform repo's `version.yaml`, and `majnet-update` on the main node
//! converges to it (ADR 0005, ADR 0015). So `status` is a comparison of three
//! things that are often not the same: what is pinned, what is published, and
//! what is actually running. Watching them converge is the whole point.

use anyhow::{bail, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use crate::client::Api;
use crate::output::{cell, elide, emit, rows, Table};
use crate::App;

#[derive(Subcommand)]
pub enum ControlPlaneCmd {
    /// Pinned vs published vs running, plus the commits in between.
    Status,
    /// Publish a new pin, or roll back to a past one (platform-admin).
    Pin {
        /// Git ref to pin. Omit when using --from-commit.
        #[arg(long = "ref", value_name = "REF")]
        git_ref: Option<String>,
        /// Control-plane image, pinned by digest (never a tag).
        #[arg(long)]
        image: Option<String>,
        /// Dashboard image, pinned by digest.
        #[arg(long)]
        dashboard: Option<String>,
        /// Roll back: copy the whole pin from this platform-repo commit.
        #[arg(long, value_name = "SHA", conflicts_with_all = ["git_ref", "image", "dashboard"])]
        from_commit: Option<String>,
    },
}

pub async fn control_plane(app: &App, cmd: ControlPlaneCmd) -> Result<()> {
    match cmd {
        ControlPlaneCmd::Status => status(app).await,
        ControlPlaneCmd::Pin {
            git_ref,
            image,
            dashboard,
            from_commit,
        } => {
            if git_ref.is_none() && from_commit.is_none() {
                bail!("pass --ref <git ref> to publish a pin, or --from-commit <sha> to roll back");
            }
            // Images are pinned by digest, never by tag — a tag makes "what is
            // running" unanswerable, which is exactly what this page exists to
            // answer. The bot rejects it too; failing here saves a round trip.
            for (flag, value) in [("--image", &image), ("--dashboard", &dashboard)] {
                if let Some(v) = value {
                    if !v.contains('@') {
                        bail!("{flag} must be pinned by digest (…@sha256:…), not by tag");
                    }
                }
            }
            let body = json!({
                "ref": git_ref,
                "image": image,
                "dashboard": dashboard,
                "from_commit": from_commit,
            });
            let message = app
                .client
                .send_text(
                    Api::Bot,
                    reqwest::Method::PUT,
                    "/api/control-plane/pin",
                    Some(&body),
                )
                .await?;
            println!("{message}");
            println!("\nThe main node converges to the new pin within ~30s: `majnet control-plane status`");
            Ok(())
        }
    }
}

async fn status(app: &App) -> Result<()> {
    let value = app.client.get_value(Api::Bot, "/api/control-plane").await?;
    emit(app.format, &value, |v| {
        let current = v.get("current").cloned().unwrap_or(Value::Null);
        let latest = v.get("latest").cloned().unwrap_or(Value::Null);
        let running = v.get("running").cloned().unwrap_or(Value::Null);

        println!("pinned    {}", cell(&current, "ref"));
        println!(
            "running   {} ({})",
            cell(&running, "version"),
            elide(&cell(&running, "commit"), 8)
        );
        // `converged: null` means the running build is unknown — a third state,
        // and printing it as "no" would send someone chasing a rollout that is
        // in fact fine.
        println!(
            "converged {}",
            match v.get("converged") {
                Some(Value::Bool(true)) => "yes".to_string(),
                Some(Value::Bool(false)) => "NO — rollout in flight".to_string(),
                _ => "unknown (the running build did not report)".to_string(),
            }
        );
        if !latest.is_null() {
            println!(
                "latest    {}{}",
                cell(&latest, "ref"),
                if v.get("latest_building")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    "  (images still building)"
                } else {
                    ""
                }
            );
        }
        if v.get("up_to_date").and_then(Value::as_bool) == Some(false) {
            println!("\nnot up to date — newer commits:");
            let mut table = Table::new(&["sha", "date", "author", "message"]);
            for c in rows(&v.get("commits").cloned().unwrap_or(Value::Null)) {
                table.push(vec![
                    elide(&cell(&c, "sha"), 8),
                    elide(&cell(&c, "date"), 19),
                    cell(&c, "author"),
                    cell(&c, "message"),
                ]);
            }
            table.print();
        }
        if let Some(error) = v.get("check_error").and_then(Value::as_str) {
            println!("\nupdate check failed: {error}");
        }
    })
}

pub async fn version(app: &App) -> Result<()> {
    // This one answers in YAML, not JSON — it is `version.yaml` verbatim.
    let text = app
        .client
        .get_text(Api::Bot, "/api/platform/version")
        .await?;
    print!("{text}");
    if !text.ends_with('\n') {
        println!();
    }
    Ok(())
}
