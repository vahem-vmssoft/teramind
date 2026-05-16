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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embeddings_migration_applies_and_view_works() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(
        dir.path().to_path_buf(), "teramind",
    ).await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    teramind_db::migrate::run(&pool).await?;

    // Table exists with the expected columns.
    let (col_count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM information_schema.columns WHERE table_name='embeddings'",
    ).fetch_one(pool.pg()).await?;
    assert_eq!(col_count, 7);

    // HNSW index present.
    let (idx_count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM pg_indexes WHERE indexname='embeddings_hnsw'",
    ).fetch_one(pool.pg()).await?;
    assert_eq!(idx_count, 1);

    // View returns zero rows on an empty corpus.
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM traces_to_embed")
        .fetch_one(pool.pg()).await?;
    assert_eq!(n, 0);

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wiki_pages_migration_applies_and_traces_fts_rebuilt() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(
        dir.path().to_path_buf(), "teramind",
    ).await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    teramind_db::migrate::run(&pool).await?;

    // wiki_pages table has 7 columns.
    let (cols,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM information_schema.columns WHERE table_name='wiki_pages'",
    ).fetch_one(pool.pg()).await?;
    assert_eq!(cols, 7);

    // sessions_to_summarize view exists.
    let (view_count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM information_schema.views WHERE table_name='sessions_to_summarize'",
    ).fetch_one(pool.pg()).await?;
    assert_eq!(view_count, 1);

    // traces_fts still queryable after the rebuild.
    let (_,): (i64,) = sqlx::query_as("SELECT count(*) FROM traces_fts")
        .fetch_one(pool.pg()).await?;

    // CASCADE: deleting a session removes its wiki_pages.
    use teramind_db::repos::{AgentRepo, SessionRepo};
    use teramind_db::repos::session::NewSession;
    use time::OffsetDateTime;
    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await?;
    let sid = sessions.insert(NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/p",
        project_id: None, parent_session_id: None,
        git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u",
        started_at: OffsetDateTime::now_utc(),
    }).await?;
    sqlx::query("INSERT INTO wiki_pages (session_id, model, content, input_tokens, output_tokens) VALUES ($1, 'm', 'x', 1, 1)")
        .bind(sid.0).execute(pool.pg()).await?;
    sqlx::query("DELETE FROM sessions WHERE id = $1").bind(sid.0).execute(pool.pg()).await?;
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM wiki_pages WHERE session_id = $1")
        .bind(sid.0).fetch_one(pool.pg()).await?;
    assert_eq!(n, 0, "CASCADE delete should have removed wiki_pages");

    sup.shutdown().await?;
    Ok(())
}
