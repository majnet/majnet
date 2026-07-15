//! Minimal rust-service with the MajNet standard endpoints (design doc §16):
//! `/healthz` (liveness — the platform's default health path) and `/info`
//! (build metadata the reconciler scrapes at deploy time and shows per env in
//! the dashboard). Build metadata is injected at image-build time via Docker
//! ARGs → ENV (see Dockerfile + the build/release workflows). Replace the
//! catch-all handler with your real service.

use axum::{routing::get, Json, Router};

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/info", get(info))
        .fallback(|| async { "rust-service is running" });

    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080u16);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .expect("bind");
    println!("rust-service listening on :{port}");
    axum::serve(listener, app).await.expect("serve");
}

/// Build metadata baked into the image at build time.
async fn info() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "version": std::env::var("APP_VERSION").unwrap_or_else(|_| "dev".into()),
        "commit": std::env::var("GIT_COMMIT").unwrap_or_else(|_| "unknown".into()),
        "build_time": std::env::var("BUILD_TIME").ok(),
    }))
}
