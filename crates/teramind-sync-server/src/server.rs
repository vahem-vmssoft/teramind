//! axum app construction + listener.

use crate::handlers;
use crate::state::AppState;
use axum::{
    extract::Path,
    http::{header, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use tower_http::trace::TraceLayer;
use tracing::info;

/// CSP for /dashboard/* responses, per
/// docs/superpowers/specs/2026-05-17-teramind-web-dashboard-design.md §8.
/// `connect-src` permits same-origin WebSocket upgrades; `frame-ancestors 'none'`
/// blocks clickjacking; inline styles are allowed because Tailwind emits some
/// `style=` attributes for dynamic values.
const DASHBOARD_CSP: &str = "default-src 'self'; \
script-src 'self'; \
style-src 'self' 'unsafe-inline'; \
connect-src 'self' ws: wss:; \
img-src 'self' data:; \
object-src 'none'; \
frame-ancestors 'none'";

fn apply_dashboard_security_headers(resp: &mut axum::response::Response) {
    resp.headers_mut().insert(
        "content-security-policy",
        HeaderValue::from_static(DASHBOARD_CSP),
    );
    resp.headers_mut()
        .insert("x-frame-options", HeaderValue::from_static("DENY"));
    resp.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
}

async fn serve_dashboard_index() -> impl IntoResponse {
    match crate::dashboard_assets::lookup("index.html") {
        Some((bytes, ct)) => {
            let mut resp = bytes.into_response();
            resp.headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static(ct));
            apply_dashboard_security_headers(&mut resp);
            resp
        }
        None => {
            let mut resp = (StatusCode::NOT_FOUND, "dashboard not built").into_response();
            apply_dashboard_security_headers(&mut resp);
            resp
        }
    }
}

async fn serve_dashboard_asset(Path(path): Path<String>) -> impl IntoResponse {
    match crate::dashboard_assets::lookup(&path) {
        Some((bytes, ct)) => {
            let mut resp = bytes.into_response();
            resp.headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static(ct));
            apply_dashboard_security_headers(&mut resp);
            resp
        }
        None => {
            let mut resp = (StatusCode::NOT_FOUND, "not found").into_response();
            apply_dashboard_security_headers(&mut resp);
            resp
        }
    }
}

pub fn build_router(state: AppState) -> Router {
    let public = Router::new()
        .route("/v1/health", get(handlers::health::health))
        .route("/v1/version", get(handlers::health::version))
        .route("/v1/auth/redeem", post(handlers::redeem::redeem))
        .route("/v1/events", get(handlers::events::events))
        .route("/dashboard", axum::routing::get(serve_dashboard_index))
        .route(
            "/dashboard/{*path}",
            axum::routing::get(serve_dashboard_asset),
        );
    let authed = Router::new()
        .route("/v1/ingest", post(handlers::ingest::ingest))
        .route("/v1/rpc", post(handlers::rpc::rpc))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::auth_middleware,
        ));

    let admin_public = Router::new()
        .route(
            "/admin/login",
            post(crate::admin_api::handlers::session::login),
        )
        .route(
            "/admin/logout",
            post(crate::admin_api::handlers::session::logout),
        )
        .route(
            "/admin/version",
            get(crate::admin_api::handlers::session::version),
        );
    let admin_authed = Router::new()
        .route("/admin/me", get(crate::admin_api::handlers::session::me))
        .route(
            "/admin/activity",
            axum::routing::get(crate::admin_api::handlers::activity::activity),
        )
        .route(
            "/admin/events",
            axum::routing::get(crate::admin_api::handlers::activity::events_ws),
        )
        .route(
            "/admin/skills",
            axum::routing::get(crate::admin_api::handlers::skills::list),
        )
        .route(
            "/admin/skills/{id}",
            axum::routing::get(crate::admin_api::handlers::skills::show)
                .delete(crate::admin_api::handlers::skills::delete),
        )
        .route(
            "/admin/candidates",
            axum::routing::get(crate::admin_api::handlers::candidates::list),
        )
        .route(
            "/admin/candidates/{id}",
            axum::routing::get(crate::admin_api::handlers::candidates::show)
                .patch(crate::admin_api::handlers::candidates::patch),
        )
        .route(
            "/admin/candidates/{id}/approve",
            axum::routing::post(crate::admin_api::handlers::candidates::approve),
        )
        .route(
            "/admin/candidates/{id}/reject",
            axum::routing::post(crate::admin_api::handlers::candidates::reject),
        )
        .route(
            "/admin/observations",
            axum::routing::get(crate::admin_api::handlers::observations::list),
        )
        .route(
            "/admin/observations/{id}",
            axum::routing::get(crate::admin_api::handlers::observations::show),
        )
        .route(
            "/admin/members",
            axum::routing::get(crate::admin_api::handlers::members::members),
        )
        .route(
            "/admin/members/{user_id}/revoke",
            axum::routing::post(crate::admin_api::handlers::members::revoke_user),
        )
        .route(
            "/admin/members/{user_id}/devices",
            axum::routing::get(crate::admin_api::handlers::members::user_devices),
        )
        .route(
            "/admin/devices/{device_id}/revoke",
            axum::routing::post(crate::admin_api::handlers::members::revoke_device),
        )
        .route(
            "/admin/invites",
            axum::routing::get(crate::admin_api::handlers::members::list_invites)
                .post(crate::admin_api::handlers::members::create_invite),
        )
        .route(
            "/admin/invites/{id}/revoke",
            axum::routing::post(crate::admin_api::handlers::members::revoke_invite),
        )
        .route(
            "/admin/quality",
            axum::routing::get(crate::admin_api::handlers::quality::list),
        )
        .route(
            "/admin/quality/latest",
            axum::routing::get(crate::admin_api::handlers::quality::latest),
        )
        .route(
            "/admin/quality/runs",
            axum::routing::post(crate::admin_api::handlers::quality::upload),
        )
        .route(
            "/admin/quality/config",
            axum::routing::get(crate::admin_api::handlers::quality::config),
        )
        .route(
            "/admin/health",
            axum::routing::get(crate::admin_api::handlers::health::health),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::admin_api::auth::admin_middleware,
        ));
    let admin = admin_public.merge(admin_authed);

    public
        .merge(authed)
        .merge(admin)
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

pub async fn serve(state: AppState, addr: SocketAddr) -> anyhow::Result<()> {
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "teramind-sync-server listening (HTTP)");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
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
        .serve(app.into_make_service())
        .await?;
    Ok(())
}
