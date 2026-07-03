//! SOPS + age secret handling (§14) — decrypt with the class key
//! (`age-production` / `age-stable`) at deploy time, inject as tmpfs-mounted
//! files at container create. Never env vars, never written to disk.

// TODO(phase-2): implement.
