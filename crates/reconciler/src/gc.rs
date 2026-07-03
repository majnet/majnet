//! Ephemeral GC (§8, §13) — remove preview stacks 48 h after PR close,
//! 7 d hard TTL regardless. Deletions only when the manifest is gone from
//! `env/ephemeral` or the TTL has expired.

// TODO(phase-4): implement.
