//! Authorization plumbing for the reconciler's human-facing endpoints
//! (restart, TTL extend). Role logic + header trust model live in
//! `majnet_common::authz`; config comes from bot snapshots. A project may be
//! addressed by name or by org here — the registry maps either to the org the
//! ops fetch needs.

use anyhow::{Context, Result};
use axum::http::HeaderMap;
use majnet_common::authz::{self, Actor};
use majnet_common::platform::{PeopleFile, ProjectsFile};
use majnet_common::project::{ProjectConfig, Role};

use crate::AppState;

/// Enforce that the caller is a platform admin (or WG-mesh infra when there's
/// no identity header). For platform-level writes like alert settings.
pub async fn require_platform_admin(state: &AppState, headers: &HeaderMap) -> Result<String> {
    let Some(login) = headers
        .get("tailscale-user-login")
        .and_then(|v| v.to_str().ok())
    else {
        return Ok("infra".into());
    };
    let platform = crate::snapshot::fetch(
        &state.http,
        &state.config,
        &state.config.root_org,
        "platform",
        "main",
    )
    .await?
    .context("platform snapshot unavailable for authz")?;
    let people = PeopleFile::parse(
        platform
            .files
            .get("people.yaml")
            .context("platform repo has no people.yaml")?,
    )?;
    match authz::identify(Some(login), &people)? {
        Actor::Human {
            github,
            platform_admin: true,
        } => Ok(github),
        _ => anyhow::bail!("platform admin required"),
    }
}

/// Like `require_platform_admin`, but for the terminal (ADR 0016): the
/// header-less WG `Infra` bypass is NOT accepted — a terminal session must be
/// attributable to a named human platform admin. Returns the github login.
pub async fn require_named_platform_admin(state: &AppState, headers: &HeaderMap) -> Result<String> {
    let login = headers
        .get("tailscale-user-login")
        .and_then(|v| v.to_str().ok())
        .context("terminal requires an authenticated platform admin (no identity header)")?;
    let platform = crate::snapshot::fetch(
        &state.http,
        &state.config,
        &state.config.root_org,
        "platform",
        "main",
    )
    .await?
    .context("platform snapshot unavailable for authz")?;
    let people = PeopleFile::parse(
        platform
            .files
            .get("people.yaml")
            .context("platform repo has no people.yaml")?,
    )?;
    match authz::identify(Some(login), &people)? {
        Actor::Human {
            github,
            platform_admin: true,
        } => Ok(github),
        _ => anyhow::bail!("platform admin required"),
    }
}

/// Enforce `min_role` on `project` for this request; returns the audit label.
pub async fn require(
    state: &AppState,
    headers: &HeaderMap,
    project: &str,
    min_role: Role,
) -> Result<String> {
    let Some(login) = headers
        .get("tailscale-user-login")
        .and_then(|v| v.to_str().ok())
    else {
        // No identity header = WG-mesh infra / node-local break-glass.
        return Ok("infra".into());
    };

    let platform = crate::snapshot::fetch(
        &state.http,
        &state.config,
        &state.config.root_org,
        "platform",
        "main",
    )
    .await?
    .context("platform snapshot unavailable for authz")?;
    let people = PeopleFile::parse(
        platform
            .files
            .get("people.yaml")
            .context("platform repo has no people.yaml")?,
    )?;
    let actor = authz::identify(Some(login), &people)?;

    let project_cfg: Option<ProjectConfig> = match &actor {
        Actor::Human {
            platform_admin: false,
            ..
        } => {
            let projects = ProjectsFile::parse(
                platform
                    .files
                    .get("projects.yaml")
                    .context("platform repo has no projects.yaml")?,
            )?;
            // Callers address a project by **org** (the dashboard and the CLI
            // both do; `/api/secrets` even fetches the ops repo straight from
            // this value), while `projects.yaml` keys on the project *name*.
            // They differ whenever an org is not named after its project, and
            // matching only on `name` denied every non-platform-admin member of
            // such a project — a 403 that read as "you have no role" when the
            // real cause was a lookup miss. Accept either.
            let org = &projects
                .projects
                .iter()
                .find(|p| p.name == project || p.org == project)
                .with_context(|| format!("unknown project {project}"))?
                .org;
            let ops = crate::snapshot::fetch(&state.http, &state.config, org, "ops", "main")
                .await?
                .with_context(|| format!("{org}/ops snapshot unavailable"))?;
            Some(
                serde_yaml::from_slice(
                    ops.files
                        .get("project.yaml")
                        .with_context(|| format!("{org}/ops has no project.yaml"))?,
                )
                .context("parsing project.yaml")?,
            )
        }
        _ => None,
    };

    authz::require_role(&actor, project_cfg.as_ref(), min_role)?;
    Ok(actor.label().to_string())
}
