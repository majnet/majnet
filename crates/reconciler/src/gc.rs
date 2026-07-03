//! Ephemeral TTL GC (§8, §13) — phase 4. Remove preview stacks 48 h after PR
//! close, 7 d hard TTL regardless.
//!
//! Note: removed-app GC (config gone from git) already happens every cycle in
//! `deploy::gc_removed_apps`; this module is only about *time-based* expiry
//! of `env/ephemeral` stacks, which needs PR-close timestamps from the bot.

// TODO(phase-4): implement.
