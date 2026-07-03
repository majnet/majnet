//! Root platform repo config (`majksa-platform/platform`) — §10.

use serde::{Deserialize, Serialize};

/// `nodes.yaml` — the three static nodes and their WireGuard endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub name: String,
    /// `main` | `prod` | `private`
    pub role: String,
    pub wireguard_ip: String,
}

/// `people.yaml` — GitHub username ↔ Tailscale identity mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Person {
    pub github: String,
    pub tailscale: String,
    #[serde(default)]
    pub admin: bool,
}

/// `projects.yaml` — the registry that gates project discovery (§2).
/// A project exists only when the GitHub App is installed on the org
/// **and** the org appears here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRegistryEntry {
    pub name: String,
    pub org: String,
}
