//! Releases (ADR 0009): the bot watches app repos' GitHub Releases, reads the
//! `majnet-release.yaml` descriptor at the tag, records it, and promotes a
//! chosen release into `ops` production (stable auto-tracks the latest tag).

use anyhow::{Context, Result};
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use majnet_common::manifest::Migration;
use majnet_common::project::Role;
use majnet_common::release::Release;
use secrecy::ExposeSecret;
use std::sync::Arc;

use axum::Json;

use crate::state::StoredRelease;
use crate::AppState;

type ApiError = (StatusCode, String);

const DESCRIPTOR: &str = "majnet-release.yaml";

/// Handle a `release` webhook: on publish, read + validate the `majnet-release.yaml`
/// **release asset** (CI computes the digests at build time, after the tag, so the
/// descriptor is an asset, not a committed file) and record it. A release without
/// that asset isn't a MajNet release — we log and move on rather than error.
pub async fn on_release(state: &AppState, org: &str, payload: &serde_json::Value) -> Result<()> {
    let action = payload["action"].as_str().unwrap_or_default();
    if !matches!(action, "published" | "released" | "edited") {
        return Ok(());
    }
    let app = payload["repository"]["name"].as_str().unwrap_or_default();
    if app.is_empty() || app == "ops" {
        return Ok(());
    }
    let release = &payload["release"];
    let tag = release["tag_name"].as_str().unwrap_or_default();
    let published_at = release["published_at"].as_str().unwrap_or_default();
    if tag.is_empty() {
        return Ok(());
    }

    // The asset's API `url` serves the raw bytes with Accept: octet-stream.
    let Some(asset_url) = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["name"].as_str() == Some(DESCRIPTOR))
        .and_then(|a| a["url"].as_str())
    else {
        tracing::info!(
            org,
            app,
            tag,
            "release has no {DESCRIPTOR} asset — skipping"
        );
        return Ok(());
    };
    let (_, token) = state.github.org_client_and_token(org).await?;
    let bytes = state
        .http
        .get(asset_url)
        .bearer_auth(token.expose_secret())
        .header(header::ACCEPT, "application/octet-stream")
        .header(header::USER_AGENT, "majnet-bot")
        .send()
        .await?
        .error_for_status()
        .with_context(|| format!("downloading {DESCRIPTOR} for {org}/{app}@{tag}"))?
        .bytes()
        .await?;
    let descriptor = Release::parse(&bytes)
        .with_context(|| format!("{org}/{app}@{tag}: invalid {DESCRIPTOR}"))?;

    state
        .store
        .upsert_release(org, app, &descriptor, published_at)?;
    state.store.log_event(
        "release-published",
        Some(org),
        &format!("{app} {} ({})", descriptor.version, &descriptor.app),
    )?;
    tracing::info!(org, app, version = %descriptor.version, "release recorded");

    // Stable auto-tracks the latest tag (ADR 0009 phase 5): re-point
    // `apps/{app}/stable.yaml` at the newest recorded release — opt-in, so only
    // if the app committed a stable overlay. The store orders by publish time,
    // so editing/re-publishing an older release won't demote stable off the
    // true latest; `production` moves only via promote.
    if let Some(latest) = state.store.releases(org, app)?.into_iter().next() {
        if crate::digest::bump_class_digest(state, org, app, &latest.app_image, "stable").await? {
            state.store.log_event(
                "digest-bump",
                Some(org),
                &format!("{app} stable → {} ({})", latest.version, latest.app_image),
            )?;
        }
    }
    Ok(())
}

/// `GET /api/releases/{org}/{app}` — recorded releases, newest first.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Path((org, app)): Path<(String, String)>,
) -> Result<Json<Vec<StoredRelease>>, (StatusCode, String)> {
    state
        .store
        .releases(&org, &app)
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

#[derive(serde::Serialize)]
struct ProdOverlay {
    image: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    migration: Option<Migration>,
}

/// `POST /api/releases/{org}/{app}/promote/{version}` — pin production to a
/// chosen release (ADR 0009): write its app + migration digests into
/// `apps/{app}/production.yaml` on ops main. Admin-gated; the `env/production`
/// render PR (the §9 gate) follows. Stable auto-tracks the latest tag, so
/// promotion targets production only.
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

    // Overlay pins the app image; migration (if any) carries its own image, or
    // omits it to run in the app image (Migration defaults on the reconciler).
    let migration = rel.migration_command.clone().map(|command| Migration {
        image: rel.migration_image.clone(),
        command,
    });
    let overlay = format!(
        "# production overlay for {app} — release {version} (ADR 0009)\n{}",
        serde_yaml::to_string(&ProdOverlay {
            image: rel.app_image.clone(),
            migration,
        })
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
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
