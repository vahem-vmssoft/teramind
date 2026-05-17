//! Verifies the team-mode migration applies cleanly on a fresh PG.

use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn team_mode_migration_creates_tables_and_columns() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    // Tables exist.
    for t in ["users", "devices", "invites"] {
        let (exists,): (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = $1)",
        )
        .bind(t)
        .fetch_one(pool.pg())
        .await?;
        assert!(exists, "table `{t}` should exist after migration");
    }

    // Additive columns are present on sessions + skills.
    for (table, col) in [
        ("sessions", "user_id"),
        ("sessions", "device_id"),
        ("skills", "user_id"),
        ("skills", "device_id"),
    ] {
        let (exists,): (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
             WHERE table_name = $1 AND column_name = $2)",
        )
        .bind(table)
        .bind(col)
        .fetch_one(pool.pg())
        .await?;
        assert!(exists, "{table}.{col} should exist after migration");
    }

    sup.shutdown().await?;
    Ok(())
}
