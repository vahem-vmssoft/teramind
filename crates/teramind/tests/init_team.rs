//! Integration test: real server + real client init flow.

use std::net::SocketAddr;
use teramind_core::team::TeamConfig;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramind_sync_server::{config::*, invite::InviteCode, server::build_router, state::AppState};
use time::{Duration as TDur, OffsetDateTime};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn init_team_redeems_and_writes_config() -> anyhow::Result<()> {
    let pg_dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(pg_dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let cfg = ServerConfig {
        listen_addr: "127.0.0.1:0".into(),
        database_url: "ignored".into(),
        tls: None,
        auth: AuthConfig::default(),
        ingest: IngestConfig::default(),
    };
    let state = AppState::new(pool.clone(), cfg);
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let invites = teramind_db::repos::InviteRepo::new(pool.clone());
    let code = InviteCode::from_bytes([0x55u8; 16]);
    invites
        .create(
            &code.hash(),
            "alice@acme.dev",
            None,
            None,
            OffsetDateTime::now_utc() + TDur::days(7),
        )
        .await?;

    let cfg_dir = tempfile::tempdir()?;
    std::env::set_var("XDG_CONFIG_HOME", cfg_dir.path());

    teramind::commands::init_team::run(
        format!("http://{addr}"),
        code.as_str().to_string(),
        Some("test-device".into()),
    )
    .await?;

    let team_toml = cfg_dir.path().join("teramind").join("team.toml");
    let cfg = TeamConfig::load(&team_toml)?;
    assert_eq!(cfg.device_name, "test-device");
    assert!(cfg.device_token.starts_with("tmd_v1_"));

    let key_path = cfg_dir.path().join("teramind").join("team-key");
    let key = teramind_core::team::load_signing_key(&key_path)?;
    assert_eq!(key.to_bytes().len(), 32);

    sup.shutdown().await?;
    Ok(())
}
