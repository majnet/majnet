//! WG-internal API (§12.8): the bot's deploy nudge + read-only state for the
//! dashboard. The phase-5 restart escape hatch (§16) will live here too.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use std::sync::Arc;

use crate::AppState;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/notify", post(notify))
        .route("/api/events", get(events))
        .with_state(state)
}

/// The bot's nudge — payload is informational; convergence always reconciles
/// everything from snapshots (idempotence over cleverness).
async fn notify(State(state): State<Arc<AppState>>, body: Json<serde_json::Value>) -> StatusCode {
    tracing::info!(payload = %body.0, "notified by bot");
    state.wakeup.notify_one();
    StatusCode::ACCEPTED
}

#[derive(serde::Deserialize)]
struct EventsQuery {
    #[serde(default = "default_limit")]
    limit: u32,
}

fn default_limit() -> u32 {
    100
}

async fn events(
    State(state): State<Arc<AppState>>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<Vec<crate::state::Event>>, (StatusCode, String)> {
    state
        .store
        .recent(query.limit.min(1000))
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}
