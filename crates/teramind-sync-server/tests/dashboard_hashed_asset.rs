//! Dashboard §5 — GET /dashboard/assets/<hash>.js returns 200 with a
//! javascript content-type. The hashed filename comes from Vite's build
//! output, which is embedded via include_dir at compile time.

use std::net::SocketAddr;
use teramind_sync_server::config::*;
use teramind_sync_server::server::build_router;
use teramind_sync_server::state::AppState;

fn admin_cfg() -> AdminConfig {
    use argon2::password_hash::{rand_core::OsRng, SaltString};
    use argon2::{Argon2, PasswordHasher};
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(b"hunter2hunter2", &salt)
        .unwrap()
        .to_string();
    AdminConfig {
        admin_password_hash: hash,
        admin_session_secret: "ab".repeat(32),
        admin_session_ttl_hours: 12,
        event_log_retention_days: 90,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serves_hashed_js_asset_with_javascript_content_type() -> anyhow::Result<()> {
    // Discover the actual hashed asset name from dashboard/dist/assets/ on disk.
    // The same directory is embedded into the binary via include_dir.
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let assets_dir = manifest.join("../../dashboard/dist/assets");
    let entries = match std::fs::read_dir(&assets_dir) {
        Ok(e) => e,
        Err(_) => {
            eprintln!(
                "SKIP: dashboard/dist/assets not present at {assets_dir:?} — \
                 skipping hashed-asset test (build artifact not bundled)"
            );
            return Ok(());
        }
    };
    let js_name = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .find(|n| n.ends_with(".js"));
    let Some(js_name) = js_name else {
        eprintln!("SKIP: no .js asset found under dashboard/dist/assets");
        return Ok(());
    };

    let pool = teramind_db::testing::fresh_pool().await?;
    let cfg = ServerConfig {
        listen_addr: "127.0.0.1:0".into(),
        database_url: "ignored".into(),
        tls: None,
        auth: AuthConfig::default(),
        ingest: IngestConfig::default(),
        admin: Some(admin_cfg()),
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
        .get(format!("http://{addr}/dashboard/assets/{js_name}"))
        .send()
        .await?;
    assert_eq!(r.status(), 200, "hashed JS asset must be served");
    let ct = r
        .headers()
        .get("content-type")
        .expect("content-type header must be set")
        .to_str()?
        .to_lowercase();
    assert!(
        ct.contains("javascript"),
        "content-type must indicate javascript, got: {ct}"
    );
    Ok(())
}
