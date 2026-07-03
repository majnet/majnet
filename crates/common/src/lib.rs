//! majnet-common — shared types for the MajNet v2 control plane.
//!
//! Home of the manifest schema (app `base.yaml` + class overlays), the
//! project config (`project.yaml`), the platform config (`nodes.yaml`,
//! `people.yaml`, `projects.yaml`) and strict validation used by both the
//! bot (at render time) and the reconciler (defensively at deploy time).

pub mod manifest;
pub mod platform;
pub mod project;

/// Environment classes — see design doc §8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnvClass {
    /// Public, gated behind a reviewed `env/production` render PR. Runs on the prod node.
    Production,
    /// VPN-only, auto-deployed on merge to main. Runs on the private node.
    Stable,
    /// VPN-only, PR-scoped preview. 48 h grace after PR close, 7 d hard TTL.
    Ephemeral,
}

impl EnvClass {
    /// Static trust-zoned placement: the node follows from the class (§3, §4).
    pub fn node_role(self) -> &'static str {
        match self {
            EnvClass::Production => "prod",
            EnvClass::Stable | EnvClass::Ephemeral => "private",
        }
    }
}
