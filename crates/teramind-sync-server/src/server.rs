//! axum app construction + listener.

use crate::handlers;
use crate::state::AppState;
use axum::{routing::{get, post}, Router};
use std::net::SocketAddr;
use tower_http::trace::TraceLayer;
use tracing::info;

pub fn build_router(state: AppState) -> Router {
    let public = Router::new()
        .route("/v1/health",      get(handlers::health::health))
        .route("/v1/version",     get(handlers::health::version))
        .route("/v1/auth/redeem", post(handlers::redeem::redeem));
    let authed = Router::new()
        .route("/v1/ingest", post(handlers::ingest::ingest))
        .layer(axum::middleware::from_fn_with_state(state.clone(), crate::auth::auth_middleware));
    public.merge(authed).with_state(state).layer(TraceLayer::new_for_http())
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
    tls: &crate::config::TlsConfig,
) -> anyhow::Result<()> {
    let app = build_router(state);
    let cfg = crate::tls::rustls_config(tls)?;
    let acceptor = axum_server::tls_rustls::RustlsConfig::from_config(cfg);
    info!(%addr, "teramind-sync-server listening (HTTPS)");
    axum_server::bind_rustls(addr, acceptor)
        .serve(app.into_make_service()).await?;
    Ok(())
}
