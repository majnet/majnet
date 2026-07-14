//! Data-preserving app/project rename (imperative, §16-adjacent). The bot does
//! the git identity change (repo rename, ops commit, render-PR merge); the
//! reconciler moves the physical data that git can't:
//!
//! - `prepare` freezes the `(project, old, new, class)` rename in the store so
//!   the drift-poll converge + GC skip both names during the window (they can't
//!   create an empty new stack or GC the still-serving old one).
//! - `commit`, called after the git flip has updated `env/<class>`, stops the
//!   old container, copies each named volume old→new, renames the managed DB
//!   old→new, then clears the freeze so the next converge brings the new stack
//!   up on the migrated data (health-gated) and GCs the old one.
//!
//! Old volumes are left behind ("archive, never delete"); the DB rename moves
//! the data in place (Postgres) or table-by-table then drops the old DB
//! (MariaDB).

use anyhow::{Context, Result};
use bollard::query_parameters as qp;
use bollard::Docker;
use futures_util::StreamExt;
use majnet_common::manifest::AppManifest;
use majnet_common::platform::{NodesFile, ProjectsFile};
use majnet_common::EnvClass;

use crate::deploy::{self, DeployCtx};
use crate::AppState;

/// Freeze the rename across every class the app is currently deployed in
/// (discovered from the pre-flip `env/<class>` branches). Returns those classes.
pub async fn prepare(state: &AppState, org: &str, old: &str, new: &str) -> Result<Vec<String>> {
    let project = resolve_project(state, org).await?;
    let mut classes = Vec::new();
    for class in EnvClass::ALL {
        let Some(snap) =
            crate::snapshot::fetch(&state.http, &state.config, org, "ops", &class.env_branch())
                .await?
        else {
            continue;
        };
        if snap.files.contains_key(&format!("{old}.yaml")) {
            state
                .store
                .rename_add_pending(&project, old, new, class.as_str())?;
            classes.push(class.as_str().to_string());
        }
    }
    Ok(classes)
}

/// Migrate data for every frozen class of this rename, then clear the freeze.
/// Must run after the git flip: it reads the *new* manifest from `env/<class>`.
pub async fn commit(state: &AppState, org: &str, old: &str, new: &str) -> Result<Vec<String>> {
    let project = resolve_project(state, org).await?;
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

    let mut done = Vec::new();
    for class in EnvClass::ALL {
        // Only classes frozen for *this* old→new rename.
        if !state
            .store
            .renames_pending(&project, class.as_str())?
            .iter()
            .any(|(o, n)| o == old && n == new)
        {
            continue;
        }
        migrate_class(state, &nodes, &project, org, old, new, class)
            .await
            .with_context(|| format!("migrating {old}→{new} ({})", class.as_str()))?;
        state.store.rename_complete(&project, old, class.as_str())?;
        state.store.record(
            "imperative-rename",
            &project,
            "",
            &format!("rename {old}→{new}"),
            class.as_str(),
        )?;
        done.push(class.as_str().to_string());
    }
    Ok(done)
}

async fn migrate_class(
    state: &AppState,
    nodes: &NodesFile,
    project: &str,
    org: &str,
    old: &str,
    new: &str,
    class: EnvClass,
) -> Result<()> {
    let node = nodes
        .by_role(class.node_role())
        .context("no node for class")?;
    let docker = state.nodes(nodes).client_for(node).await?;

    // New manifest (post-flip env branch) tells us which volumes + DB to move.
    let snap = crate::snapshot::fetch(&state.http, &state.config, org, "ops", &class.env_branch())
        .await?
        .with_context(|| format!("env/{} snapshot", class.as_str()))?;
    let bytes = snap
        .files
        .get(&format!("{new}.yaml"))
        .with_context(|| format!("{new}.yaml missing on env/{}", class.as_str()))?;
    let manifest = AppManifest::parse(std::str::from_utf8(bytes)?)?;

    let ctx = DeployCtx {
        docker: &docker,
        project,
        class,
        commit: "imperative-rename",
        dry_run: state.config.dry_run,
        http: &state.http,
        bot_url: &state.config.bot_url,
    };

    // Stop the old stack so the volume is quiescent and the DB has no live
    // connections (Postgres ALTER DATABASE RENAME needs none). Data lives in
    // the named volume + engine, not the container, so removal is safe.
    deploy::remove_app(&ctx, old).await?;

    for vol in &manifest.volumes {
        let from = deploy::volume_name(project, old, class, &vol.name);
        let to = deploy::volume_name(project, new, class, &vol.name);
        copy_volume(&docker, &from, &to)
            .await
            .with_context(|| format!("copying volume {from} → {to}"))?;
    }

    if let Some(db) = &manifest.database {
        crate::db::rename_database(
            &docker,
            db.engine,
            &crate::db::db_name(project, old, class),
            &crate::db::db_name(project, new, class),
        )
        .await?;
    }
    Ok(())
}

/// Copy a named Docker volume's contents into another (created if absent) via a
/// throwaway helper — the same busybox pattern as the metrics host probe.
async fn copy_volume(docker: &Docker, from: &str, to: &str) -> Result<()> {
    ensure_helper_image(docker).await;
    docker
        .create_volume(bollard::models::VolumeCreateRequest {
            name: Some(to.to_string()),
            ..Default::default()
        })
        .await
        .with_context(|| format!("creating volume {to}"))?;

    let helper = docker
        .create_container(
            None::<qp::CreateContainerOptions>,
            bollard::models::ContainerCreateBody {
                image: Some(crate::secrets::HELPER_IMAGE.into()),
                // `/.` copies contents (incl. dotfiles) rather than the dir itself.
                cmd: Some(vec![
                    "sh".into(),
                    "-c".into(),
                    format!("cp -a /from/. /to/ 2>/dev/null; true"),
                ]),
                host_config: Some(bollard::models::HostConfig {
                    binds: Some(vec![
                        format!("{from}:/from:ro"),
                        format!("{to}:/to"),
                    ]),
                    ..Default::default()
                }),
                labels: Some([("majnet.helper".to_string(), "rename".to_string())].into()),
                ..Default::default()
            },
        )
        .await
        .context("creating copy helper")?;

    let run = async {
        docker
            .start_container(&helper.id, None::<qp::StartContainerOptions>)
            .await?;
        let mut wait = docker.wait_container(&helper.id, None::<qp::WaitContainerOptions>);
        while let Some(next) = wait.next().await {
            // A non-zero exit surfaces as an Err here.
            next.context("volume copy helper failed")?;
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;

    let _ = docker
        .remove_container(
            &helper.id,
            Some(qp::RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await;
    run
}

async fn ensure_helper_image(docker: &Docker) {
    if docker
        .inspect_image(crate::secrets::HELPER_IMAGE)
        .await
        .is_err()
    {
        let _ = docker
            .create_image(
                Some(qp::CreateImageOptions {
                    from_image: Some(crate::secrets::HELPER_IMAGE.into()),
                    ..Default::default()
                }),
                None,
                None,
            )
            .collect::<Vec<_>>()
            .await;
    }
}

/// Resolve a GitHub org to its project name (container/volume/DB prefix) via the
/// platform registry — the org and the project name can differ (§2).
async fn resolve_project(state: &AppState, org: &str) -> Result<String> {
    let platform = crate::snapshot::fetch(
        &state.http,
        &state.config,
        &state.config.root_org,
        "platform",
        "main",
    )
    .await?
    .context("platform snapshot unavailable")?;
    let projects: ProjectsFile =
        serde_yaml::from_slice(platform.files.get("projects.yaml").context("no projects.yaml")?)?;
    projects
        .projects
        .into_iter()
        .find(|p| p.org == org)
        .map(|p| p.name)
        .with_context(|| format!("org '{org}' is not in the project registry"))
}
