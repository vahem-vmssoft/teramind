//! axum app construction + listener.

use crate::handlers;
use crate::state::AppState;
use axum::{routing::{get, post}, Router};
use std::net::SocketAddr;
use tower_http::trace::TraceLayer;
use tracing::info;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/health",        get(handlers::health::health))
        .route("/v1/version",       get(handlers::health::version))
        .route("/v1/auth/redeem",   post(handlers::redeem::redeem))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

pub async fn serve(state: AppState, addr: SocketAddr) -> anyhow::Result<()> {
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "teramind-sync-server listening (HTTP)");
    axum::serve(listener, app).await?;
    Ok(())
}

pub async fn serve_tls(
    state: AppState,
    addr: SocketAddr,
    _tls: &crate::config::TlsConfig,
) -> anyhow::Result<()> {
    // Replaced with real rustls/axum-server wiring in §20.
    serve(state, addr).await
}
