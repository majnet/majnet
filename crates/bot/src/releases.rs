//! Releases (ADR 0009): a **release is a `vX.Y.Z`-tagged image publish**. The
//! app's CI builds + pushes `ghcr.io/<org>/<app>:vX.Y.Z` by digest; the
//! `registry_package` webhook (which already drives the testing/ephemeral
//! digest bumps) carries the tag + digest, and the bot records it here. There
//! is no separate release descriptor — the digest comes off the webhook and the
//! migration lives in the ops overlay (`base.yaml`), next to the DB/secret
//! config it depends on. `stable` auto-tracks the latest tag; `promote` pins a
//! chosen version into `production.yaml`.

use anyhow::{Context, Result};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use majnet_common::project::Role;
use std::sync::Arc;

use crate::state::StoredRelease;
use crate::AppState;

type ApiError = (StatusCode, String);

/// Record a `vX.Y.Z` release seen on a `registry_package` publish: resolve the
/// tag's commit (best-effort provenance), store it, and re-point stable at the
/// newest tag. `app_image` is the digest-pinned `ghcr.io/<org>/<app>@sha256:…`.
pub async fn record(
    state: &AppState,
    org: &str,
    app: &str,
    version: &str,
    app_image: &str,
) -> Result<()> {
    let commit = resolve_commit(state, org, app, version)
        .await
        .unwrap_or_default();
    state
        .store
        .upsert_release(org, app, version, &commit, app_image)?;
    state.store.log_event(
        "release-published",
        Some(org),
        &format!("{app} {version} ({app_image})"),
    )?;
    tracing::info!(org, app, version, "release recorded");
    track_stable(state, org, app).await
}

/// Resolve a tag to its commit SHA via the commits API, which follows both
/// lightweight and annotated tags. Best-effort — provenance, not correctness.
async fn resolve_commit(state: &AppState, org: &str, app: &str, tag: &str) -> Result<String> {
    let client = state.github.org_client(org).await?;
    let commit: serde_json::Value = client
        .get(format!("/repos/{org}/{app}/commits/{tag}"), None::<&()>)
        .await?;
    commit["sha"]
        .as_str()
        .map(String::from)
        .context("commit lookup returned no sha")
}

/// Re-point `apps/{app}/stable.yaml` at the newest recorded release (ADR 0009
/// phase 5). Opt-in via overlay-presence; a no-op when stable is already
/// current or the app has no releases. The store orders by publish time, so
/// stable stays on the true latest; `production` moves only via promote.
async fn track_stable(state: &AppState, org: &str, app: &str) -> Result<()> {
    let Some(latest) = state.store.releases(org, app)?.into_iter().next() else {
        return Ok(());
    };
    if crate::digest::bump_class_digest(state, org, app, &latest.app_image, "stable").await? {
        state.store.log_event(
            "digest-bump",
            Some(org),
            &format!("{app} stable → {} ({})", latest.version, latest.app_image),
        )?;
    }
    Ok(())
}

/// `GET /api/releases/{org}/{app}` — recorded releases, newest first.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Path((org, app)): Path<(String, String)>,
) -> Result<Json<Vec<StoredRelease>>, ApiError> {
    state
        .store
        .releases(&org, &app)
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// `POST /api/releases/{org}/{app}/promote/{version}` — pin production to a
/// chosen release (ADR 0009): write its app digest into
/// `apps/{app}/production.yaml` on ops main. The migration is inherited from
/// `base.yaml` (version-independent command; the files travel in the image), so
/// the overlay pins only the image. Admin-gated; the `env/production` render PR
/// (the §9 gate) follows. Stable auto-tracks the latest tag, so promotion
/// targets production only.
pub async fn promote(
    State(state): State<Arc<AppState>>,
    Path((org, app, version)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<String, ApiError> {
    let actor = crate::authz::require(&state, &headers, &org, Role::Admin)
        .await
        .map_err(|e| (StatusCode::FORBIDDEN, format!("{e:#}")))?;

    let rel = state
        .store
        .releases(&org, &app)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .into_iter()
        .find(|r| r.version == version)
        .ok_or((
            StatusCode::NOT_FOUND,
            format!("release {version} not found for {app}"),
        ))?;

    let overlay = format!(
        "# production overlay for {app} — release {version} (ADR 0009)\nimage: {}\n",
        rel.app_image
    );

    // Validate base ⊕ this overlay before committing.
    let mut files = crate::dashboard_api::app_files(&state, &org, &app)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
    files.insert("production.yaml".to_string(), overlay.clone());
    crate::dashboard_api::validate_app_files(&app, &files)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("{e:#}")))?;

    crate::dashboard_api::commit_file(
        &state,
        &org,
        &format!("apps/{app}/production.yaml"),
        &overlay,
        &format!("promote({app}): release {version} to production by {actor}"),
    )
    .await
    .map_err(|e| (StatusCode::BAD_GATEWAY, format!("{e:#}")))?;

    state
        .store
        .log_event(
            "promote-release",
            Some(&org),
            &format!("{app} {version} by {actor}"),
        )
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(format!(
        "{app} {version} promoted; review the env/production render PR to deploy"
    ))
}
