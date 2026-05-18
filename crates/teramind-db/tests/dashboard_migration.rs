#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn migration_creates_tables() -> anyhow::Result<()> {
    let pool = teramind_db::testing::fresh_pool().await?;

    for t in ["team_event_log", "quality_runs"] {
        let (exists,): (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = $1)",
        )
        .bind(t)
        .fetch_one(pool.pg())
        .await?;
        assert!(exists, "table `{t}` must exist after migration");
    }

    Ok(())
}
