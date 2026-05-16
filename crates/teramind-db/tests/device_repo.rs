use teramind_db::repos::{DeviceRepo, UserRepo};
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};

async fn fresh_pool() -> anyhow::Result<(tempfile::TempDir, teramind_db::pg_supervisor::PgSupervisor, DbPool)> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;
    Ok((dir, sup, pool))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn insert_and_get_by_token_hash_round_trips() -> anyhow::Result<()> {
    let (_dir, sup, pool) = fresh_pool().await?;
    let users = UserRepo::new(pool.clone());
    let devices = DeviceRepo::new(pool.clone());

    let u = users.upsert_by_email("alice@acme.dev", None).await?;
    let token_hash = vec![0xAAu8; 32];
    let public_key = vec![0xBBu8; 32];
    let d = devices.insert(u.id, "alice-macbook", &token_hash, &public_key).await?;
    let by_hash = devices.get_active_by_token_hash(&token_hash).await?
        .expect("device must be findable by token hash");
    assert_eq!(by_hash.id, d.id);
    assert_eq!(by_hash.public_key, public_key);

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoke_excludes_from_active_lookup() -> anyhow::Result<()> {
    let (_dir, sup, pool) = fresh_pool().await?;
    let users = UserRepo::new(pool.clone());
    let devices = DeviceRepo::new(pool.clone());

    let u = users.upsert_by_email("bob@acme.dev", None).await?;
    let th = vec![0x01u8; 32]; let pk = vec![0x02u8; 32];
    let d = devices.insert(u.id, "bob-laptop", &th, &pk).await?;
    devices.revoke(d.id).await?;
    let active = devices.get_active_by_token_hash(&th).await?;
    assert!(active.is_none(), "revoked device must not appear via get_active_by_token_hash");

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn touch_last_seen_advances() -> anyhow::Result<()> {
    let (_dir, sup, pool) = fresh_pool().await?;
    let users = UserRepo::new(pool.clone());
    let devices = DeviceRepo::new(pool.clone());

    let u = users.upsert_by_email("carol@acme.dev", None).await?;
    let th = vec![0x03u8; 32]; let pk = vec![0x04u8; 32];
    let d = devices.insert(u.id, "carol-pc", &th, &pk).await?;
    let before = devices.get_active_by_token_hash(&th).await?.unwrap().last_seen_at;
    assert!(before.is_none(), "fresh device has null last_seen_at");
    devices.touch_last_seen(d.id).await?;
    let after = devices.get_active_by_token_hash(&th).await?.unwrap().last_seen_at;
    assert!(after.is_some(), "touch_last_seen must populate last_seen_at");

    sup.shutdown().await?;
    Ok(())
}
