//! Org reconciliation loop (§11.2) — hourly + on config change.
//!
//! For each org that passes the discovery gate (App installed **and** listed
//! in `projects.yaml`): ensure the `ops` repo exists, create missing app repos
//! from templates, archive removed ones (never delete), enforce settings,
//! branch protection and webhooks, sync teams + membership.

// TODO(phase-3): implement.
