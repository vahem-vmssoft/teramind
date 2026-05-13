use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use tempfile::tempdir;

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
