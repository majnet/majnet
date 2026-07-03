//! Read-only state API for the dashboard and bot (§12.8, §16): per-project
//! deploys, env inventory, health, events, diffs.
//!
//! Plus the one imperative escape hatch (§16): a narrow authenticated
//! endpoint for restart / redeploy-same-digest, audit-logged with the acting
//! Tailscale identity. Nothing else is imperative.

// TODO(phase-5): implement.
