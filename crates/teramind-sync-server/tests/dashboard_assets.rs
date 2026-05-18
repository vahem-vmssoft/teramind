use std::net::SocketAddr;
use teramind_sync_server::config::*;
use teramind_sync_server::server::build_router;
use teramind_sync_server::state::AppState;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dashboard_index_returns_html() -> anyhow::Result<()> {
    let pool = teramind_db::testing::fresh_pool().await?;

    let cfg = ServerConfig {
        listen_addr: "127.0.0.1:0".into(),
        database_url: "ignored".into(),
        tls: None,
        auth: AuthConfig::default(),
        ingest: IngestConfig::default(),
        admin: None,
        quality: None,
    };
    let state = AppState::new(pool, cfg);
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    let r = reqwest::Client::new()
        .get(format!("http://{addr}/dashboard"))
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    let ct = r
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()?
        .to_string();
    assert!(ct.starts_with("text/html"));

    let r2 = reqwest::Client::new()
        .get(format!("http://{addr}/dashboard/unknown-route"))
        .send()
        .await?;
    assert_eq!(r2.status(), 200);
    assert!(r2
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()?
        .starts_with("text/html"));

    Ok(())
}
