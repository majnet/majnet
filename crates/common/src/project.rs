//! Project `ops` repo config (`project.yaml`) — §9.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    pub name: String,
    pub members: Vec<Member>,
    pub apps: Vec<AppDecl>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    /// GitHub username — the identity everywhere (GitHub teams + Tailscale ACLs).
    pub user: String,
    pub role: Role,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Production actions, member management, secrets recipient.
    Admin,
    /// Stable/ephemeral actions only.
    Developer,
}

/// An app declared in `project.yaml`. The bot materializes the repo from the
/// named template if it is missing; removing the entry archives the repo.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppDecl {
    pub name: String,
    pub template: String,
}
