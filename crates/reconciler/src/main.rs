//! MajNet Reconciler — the single orchestrator (design doc §12).
//!
//! Consumes rendered `env/<class>` branch snapshots from the bot (it holds no
//! GitHub credentials), resolves static node placement from the environment
//! class, decrypts SOPS secrets with class keys into tmpfs mounts, and
//! converges each node's Docker API over WireGuard (bollard, mTLS).
//!
//! Deploys are blue-green: start new → health check → flip Traefik label →
//! stop old. A failed health check leaves the old container serving.
//!
//! Principles: idempotent; dry-run mode; every action tagged with its causing
//! commit; deletions only when config is gone from git; failed decrypt or
//! validation aborts that app loudly — no partial applies.
//!
//! Credentials held: age keys + Docker API mTLS certs. Nothing else.

mod converge;
mod deploy;
mod gc;
mod secrets;
mod state_api;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("majnet-reconciler starting (Phase 2 MVP — see docs/roadmap.md)");

    // TODO(phase-2): event loop — on bot notification or ~5 min poll tick,
    // fetch snapshots, diff vs Docker state, converge, record events.

    Ok(())
}
