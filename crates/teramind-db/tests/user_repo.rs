use teramind_db::repos::UserRepo;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};

async fn fresh_pool() -> anyhow::Result<(
    tempfile::TempDir,
    teramind_db::pg_supervisor::PgSupervisor,
    DbPool,
)> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;
    Ok((dir, sup, pool))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upsert_creates_then_returns_existing() -> anyhow::Result<()> {
    let (_dir, sup, pool) = fresh_pool().await?;
    let users = UserRepo::new(pool.clone());

    let a = users
        .upsert_by_email("alice@acme.dev", Some("Alice K."))
        .await?;
    let b = users
        .upsert_by_email("alice@acme.dev", Some("Alice K."))
        .await?;
    assert_eq!(a.id, b.id, "upsert must be idempotent by email");
    assert_eq!(a.email, "alice@acme.dev");

    let none = users.get_by_id(a.id).await?;
    assert!(none.is_some(), "get_by_id should round-trip");

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoke_sets_revoked_at_and_get_filters() -> anyhow::Result<()> {
    let (_dir, sup, pool) = fresh_pool().await?;
    let users = UserRepo::new(pool.clone());

    let u = users.upsert_by_email("bob@acme.dev", None).await?;
    users.revoke(u.id).await?;
    let active = users.get_active(u.id).await?;
    assert!(
        active.is_none(),
        "revoked user must not appear via get_active"
    );

    sup.shutdown().await?;
    Ok(())
}
