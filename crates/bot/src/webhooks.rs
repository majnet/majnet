//! GitHub webhook intake (§11.3) — pushes, PR open/close/sync, reviews,
//! GHA digest notifications, across all project orgs plus the root platform org.
//!
//! Pushes to `env/<class>` branches (= merged render PRs) trigger a
//! reconciler notification over the WG-internal API.

// TODO(phase-1): axum routes + signature verification.
