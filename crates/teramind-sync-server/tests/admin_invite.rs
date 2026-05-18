//! Black-box test of the admin module against an embedded PG.

use teramind_db::repos::InviteRepo;
use teramind_sync_server::admin::{invite_create, invite_revoke, AdminCtx};
use teramind_sync_server::config::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn create_then_list_then_revoke() -> anyhow::Result<()> {
    let pool = teramind_db::testing::fresh_pool().await?;

    let cfg = ServerConfig {
        listen_addr: "x".into(),
        database_url: "x".into(),
        tls: None,
        auth: AuthConfig::default(),
        ingest: IngestConfig::default(),
        admin: None,
        quality: None,
    };
    let ctx = AdminCtx {
        pool: pool.clone(),
        cfg,
    };

    invite_create(
        &ctx,
        "alice@acme.dev",
        Some("Alice"),
        Some("admin"),
        Some(7),
    )
    .await?;
    let outstanding = InviteRepo::new(pool.clone()).list_outstanding().await?;
    assert_eq!(outstanding.len(), 1);

    invite_revoke(&ctx, &outstanding[0].id.0.to_string()).await?;
    let outstanding = InviteRepo::new(pool.clone()).list_outstanding().await?;
    assert_eq!(
        outstanding.len(),
        0,
        "revoked invite must drop from outstanding list"
    );

    Ok(())
}
