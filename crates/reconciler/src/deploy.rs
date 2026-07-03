//! Blue-green deploy state machine (§12.5, docs/diagrams/lifecycles.puml):
//!
//! Migrating → Starting → HealthCheck → Flipping → Draining → Done
//!                              ↘ Failed (old container keeps serving)
//!
//! Migrations run as one-shot containers before rollout; a non-zero exit
//! fails the deploy before the new container ever starts.

// TODO(phase-2): implement.
