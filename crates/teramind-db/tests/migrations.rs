use tempfile::tempdir;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pgvector_extension_is_installable() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(
        dir.path().to_path_buf(),
        "teramind",
    )
    .await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS vector")
        .execute(pool.pg())
        .await?;
    let (version,): (String,) =
        sqlx::query_as("SELECT extversion FROM pg_extension WHERE extname='vector'")
            .fetch_one(pool.pg())
            .await?;
    assert!(version.starts_with("0."), "got {version}");
    sup.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn migrations_apply_cleanly_on_empty_db() {
    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().to_path_buf(), "teramind_test")
        .await
        .unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT extname FROM pg_extension WHERE extname IN ('pgcrypto','pg_trgm') ORDER BY extname",
    )
    .fetch_all(pool.pg())
    .await
    .unwrap();
    assert_eq!(
        rows.iter().map(|(n,)| n.as_str()).collect::<Vec<_>>(),
        vec!["pg_trgm", "pgcrypto"]
    );
    sup.shutdown().await.unwrap();
}

#[tokio::test]
async fn traces_fts_view_exists_and_is_queryable() {
    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().to_path_buf(), "teramind_test")
        .await
        .unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();
    // The view should be empty but queryable.
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM traces_fts")
        .fetch_one(pool.pg())
        .await
        .unwrap();
    assert_eq!(n, 0);
    sup.shutdown().await.unwrap();
}
