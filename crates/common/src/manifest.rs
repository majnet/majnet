//! Manifest schema v1 — per-app `base.yaml` merged with a thin class overlay.
//!
//! Rendering (base ⊕ overlay) is done by the bot; the reconciler consumes only
//! the final manifests from the `env/<class>` branches and re-validates
//! defensively. Secrets pass through SOPS-encrypted — rendering never decrypts.

use serde::{Deserialize, Serialize};

/// A rendered application manifest as it appears on an `env/<class>` branch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppManifest {
    pub name: String,
    /// Image pinned by digest (`ghcr.io/<org>/<app>@sha256:...`). Tags are not allowed.
    pub image: String,
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub secrets: Vec<String>,
    #[serde(default)]
    pub ingress: Option<Ingress>,
    #[serde(default)]
    pub health: Option<HealthCheck>,
    #[serde(default)]
    pub migration: Option<Migration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ingress {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthCheck {
    pub path: String,
    pub port: u16,
    #[serde(default = "default_retries")]
    pub retries: u32,
}

fn default_retries() -> u32 {
    5
}

/// One-shot migration container run before the blue-green rollout (§12).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Migration {
    pub command: Vec<String>,
}
