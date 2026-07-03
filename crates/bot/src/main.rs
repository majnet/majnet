//! MajNet GitHub Bot — the only component talking to GitHub and Tailscale APIs.
//!
//! Responsibilities (design doc §11):
//!  1. GitHub App auth (JWT → per-org installation tokens)
//!  2. Org reconciliation loop — repos, settings, teams, members, webhooks
//!  3. Webhook intake across all project orgs
//!  4. Digest bumps — signed commits to project `ops` repos
//!  5. Manifest rendering — base ⊕ overlay → render PRs onto `env/<class>` branches
//!  6. Tailscale sync — groups, ACLs, ingress auth keys
//!  7. PR feedback — preview URLs + deploy status
//!  8. Repo access proxy — cached snapshots served to the reconciler over WG
//!  9. Dashboard write API — UI actions → validated commits/PRs
//!
//! Credentials held: GitHub App key + Tailscale API key. Nothing else.

mod org_sync;
mod render;
mod tailscale;
mod webhooks;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("majnet-bot starting (Phase 1 MVP — see docs/roadmap.md)");

    // TODO(phase-1): GitHub App auth, webhook server (axum), digest bumps, repo proxy.
    // TODO(phase-3): org reconciliation loop, Tailscale sync.

    Ok(())
}
