//! Dashboard write API (§16, phase 5): manifest editing + member management.
//! Every write is a bot-authored commit on ops `main` — through git, never
//! around it; the render pipeline propagates from there. Role-gated via
//! `authz` (production overlay + members = project admin, rest = developer).

use anyhow::{Context, Result};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use majnet_common::manifest::AppManifest;
use majnet_common::merge::merge;
use majnet_common::project::{Member, ProjectConfig, Role};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::AppState;

type ApiError = (StatusCode, String);

fn bad_gateway(e: anyhow::Error) -> ApiError {
    (StatusCode::BAD_GATEWAY, format!("{e:#}"))
}
fn bad_request(msg: impl Into<String>) -> ApiError {
    (StatusCode::BAD_REQUEST, msg.into())
}

const MANIFEST_FILES: [&str; 4] = [
    "base.yaml",
    "stable.yaml",
    "production.yaml",
    "ephemeral.yaml",
];

/// `GET /api/manifest/{org}/{app}` — the app's manifest files on ops `main`.
pub async fn manifest_get(
    State(state): State<Arc<AppState>>,
    Path((org, app)): Path<(String, String)>,
) -> Result<Json<BTreeMap<String, String>>, ApiError> {
    check_name(&app)?;
    let files = app_files(&state, &org, &app).await.map_err(bad_gateway)?;
    Ok(Json(files))
}

/// `PUT /api/manifest/{org}/{app}/{file}` — validate + commit one manifest
/// file. Body is the raw YAML.
pub async fn manifest_put(
    State(state): State<Arc<AppState>>,
    Path((org, app, file)): Path<(String, String, String)>,
    headers: HeaderMap,
    body: String,
) -> Result<String, ApiError> {
    check_name(&app)?;
    if !MANIFEST_FILES.contains(&file.as_str()) {
        return Err(bad_request(format!(
            "file must be one of {MANIFEST_FILES:?}"
        )));
    }
    // The production overlay is a production action (§9: role admin).
    let min_role = if file == "production.yaml" {
        Role::Admin
    } else {
        Role::Developer
    };
    let actor = crate::authz::require(&state, &headers, &org, min_role)
        .await
        .map_err(|e| (StatusCode::FORBIDDEN, format!("{e:#}")))?;

    let mut files = app_files(&state, &org, &app).await.map_err(bad_gateway)?;
    files.insert(file.clone(), body.clone());
    validate_app_files(&app, &files).map_err(|e| bad_request(format!("{e:#}")))?;

    let path = format!("apps/{app}/{file}");
    let message = format!("manifest({app}): edit {file} via dashboard by {actor}");
    commit_file(&state, &org, &path, &body, &message)
        .await
        .map_err(bad_gateway)?;
    state
        .store
        .log_event("manifest-edit", Some(&org), &format!("{path} by {actor}"))
        .map_err(bad_gateway)?;
    Ok(format!(
        "{path} committed; render PRs will propagate the change"
    ))
}

/// `GET /api/members/{org}` — project.yaml members.
pub async fn members_get(
    State(state): State<Arc<AppState>>,
    Path(org): Path<String>,
) -> Result<Json<Vec<Member>>, ApiError> {
    let project = read_project(&state, &org).await.map_err(bad_gateway)?;
    Ok(Json(project.members))
}

#[derive(Deserialize)]
pub struct MemberChange {
    pub user: String,
    /// `admin` | `developer` | `remove`.
    pub role: String,
}

/// `POST /api/members/{org}` — upsert or remove one member (admin-only).
/// The bot's org sync propagates teams + Tailscale ACLs from the commit.
pub async fn members_post(
    State(state): State<Arc<AppState>>,
    Path(org): Path<String>,
    headers: HeaderMap,
    Json(change): Json<MemberChange>,
) -> Result<String, ApiError> {
    let actor = crate::authz::require(&state, &headers, &org, Role::Admin)
        .await
        .map_err(|e| (StatusCode::FORBIDDEN, format!("{e:#}")))?;
    if change.user.is_empty()
        || !change
            .user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(bad_request("invalid GitHub username"));
    }

    let mut project = read_project(&state, &org).await.map_err(bad_gateway)?;
    let action = match change.role.as_str() {
        "remove" => {
            let before = project.members.len();
            project.members.retain(|m| m.user != change.user);
            if project.members.len() == before {
                return Err(bad_request(format!("{} is not a member", change.user)));
            }
            format!("remove {}", change.user)
        }
        role @ ("admin" | "developer") => {
            let parsed: Role = serde_yaml::from_str(role).expect("checked");
            match project.members.iter_mut().find(|m| m.user == change.user) {
                Some(member) => member.role = parsed,
                None => project.members.push(Member {
                    user: change.user.clone(),
                    role: parsed,
                }),
            }
            format!("{} → {role}", change.user)
        }
        other => {
            return Err(bad_request(format!(
                "role must be admin|developer|remove, got {other}"
            )))
        }
    };

    let yaml = serde_yaml::to_string(&project).map_err(|e| bad_gateway(e.into()))?;
    let message = format!("members: {action} via dashboard by {actor}");
    commit_file(&state, &org, "project.yaml", &yaml, &message)
        .await
        .map_err(bad_gateway)?;
    state
        .store
        .log_event("member-change", Some(&org), &format!("{action} by {actor}"))
        .map_err(bad_gateway)?;
    Ok(format!("{action} committed; org sync will propagate"))
}

// ── helpers ────────────────────────────────────────────────────────────────

fn check_name(app: &str) -> Result<(), ApiError> {
    if app.is_empty()
        || !app
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(bad_request("invalid app name"));
    }
    Ok(())
}

/// The app's manifest files from the ops `main` snapshot.
async fn app_files(state: &AppState, org: &str, app: &str) -> Result<BTreeMap<String, String>> {
    let (_, tar) = crate::proxy::fetch_snapshot(state, org, "ops", "main").await?;
    let sources = majnet_common::tarball::untar(&tar)?;
    let prefix = format!("apps/{app}/");
    let mut files = BTreeMap::new();
    for (path, bytes) in sources {
        if let Some(name) = path.strip_prefix(&prefix) {
            if MANIFEST_FILES.contains(&name) {
                files.insert(name.to_string(), String::from_utf8(bytes)?);
            }
        }
    }
    Ok(files)
}

/// Validate the app's files as the render pipeline would see them after the
/// change: every present overlay must merge with base into a valid manifest.
fn validate_app_files(app: &str, files: &BTreeMap<String, String>) -> Result<()> {
    let base_str = files
        .get("base.yaml")
        .context("the app has no base.yaml — create it first")?;
    let base: serde_yaml::Value = serde_yaml::from_str(base_str).context("base.yaml")?;
    let overlays: Vec<&str> = files
        .keys()
        .map(String::as_str)
        .filter(|f| *f != "base.yaml")
        .collect();
    anyhow::ensure!(
        !overlays.is_empty(),
        "no class overlay present — the app would not render into any class"
    );
    for overlay_file in overlays {
        let overlay: serde_yaml::Value =
            serde_yaml::from_str(&files[overlay_file]).with_context(|| overlay_file.to_string())?;
        let mut merged = merge(base.clone(), overlay);
        // Same name handling as render.rs: directory is the identity.
        if let serde_yaml::Value::Mapping(map) = &mut merged {
            let key = serde_yaml::Value::from("name");
            if map.get(&key).is_none() {
                map.insert(key, serde_yaml::Value::from(app));
            }
        }
        let yaml = serde_yaml::to_string(&merged)?;
        AppManifest::parse(&yaml)
            .with_context(|| format!("base.yaml ⊕ {overlay_file} is not a valid manifest"))?;
    }
    Ok(())
}

/// Create-or-update one file on ops `main`.
async fn commit_file(
    state: &AppState,
    org: &str,
    path: &str,
    content: &str,
    message: &str,
) -> Result<()> {
    let client = state.github.org_client(org).await?;
    let repos = client.repos(org, "ops");
    match crate::promote::read_file(&repos, path).await? {
        Some((current, sha)) => {
            if current == content {
                return Ok(());
            }
            repos
                .update_file(path, message, content, &sha)
                .branch("main")
                .send()
                .await?;
        }
        None => {
            repos
                .create_file(path, message, content)
                .branch("main")
                .send()
                .await?;
        }
    }
    Ok(())
}

async fn read_project(state: &AppState, org: &str) -> Result<ProjectConfig> {
    let (_, tar) = crate::proxy::fetch_snapshot(state, org, "ops", "main").await?;
    let files = majnet_common::tarball::untar(&tar)?;
    let yaml = files
        .get("project.yaml")
        .with_context(|| format!("{org}/ops has no project.yaml"))?;
    serde_yaml::from_slice(yaml).context("parsing project.yaml")
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "image: ghcr.io/x/app@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nhealth:\n  path: /\n  port: 80\n";

    #[test]
    fn valid_base_plus_overlay_passes() {
        let files = BTreeMap::from([
            ("base.yaml".to_string(), BASE.to_string()),
            ("stable.yaml".to_string(), "env:\n  X: \"1\"\n".to_string()),
        ]);
        validate_app_files("myapp", &files).unwrap();
    }

    #[test]
    fn tag_pinned_image_is_rejected() {
        let files = BTreeMap::from([
            (
                "base.yaml".to_string(),
                "image: ghcr.io/x/app:latest\nhealth:\n  path: /\n  port: 80\n".to_string(),
            ),
            ("stable.yaml".to_string(), "{}\n".to_string()),
        ]);
        let err = validate_app_files("myapp", &files).unwrap_err();
        assert!(format!("{err:#}").contains("digest-pinned"), "{err:#}");
    }

    #[test]
    fn overlay_without_base_is_rejected() {
        let files = BTreeMap::from([("stable.yaml".to_string(), "{}\n".to_string())]);
        assert!(validate_app_files("myapp", &files).is_err());
    }

    #[test]
    fn base_without_any_overlay_is_rejected() {
        let files = BTreeMap::from([("base.yaml".to_string(), BASE.to_string())]);
        assert!(validate_app_files("myapp", &files).is_err());
    }
}
