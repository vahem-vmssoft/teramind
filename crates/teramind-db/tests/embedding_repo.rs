use teramind_db::repos::{AgentRepo, EmbeddingRepo, SessionRepo, TraceRepo};
use teramind_db::repos::session::NewSession;
use teramind_core::ids::TurnId;
use time::OffsetDateTime;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embedding_repo_bulk_insert_and_backlog() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(
        dir.path().to_path_buf(), "teramind",
    ).await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    teramind_db::migrate::run(&pool).await?;

    // Insert a session + turn so the view has a row.
    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await?;
    let sid = sessions.insert(NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/p",
        project_id: None, parent_session_id: None,
        git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u",
        started_at: OffsetDateTime::now_utc(),
    }).await?;
    let tid = trace.upsert_turn_with_id(
        TurnId(uuid::Uuid::new_v4()), sid, 0,
        OffsetDateTime::now_utc(), Some("hello world"),
    ).await?;

    let repo = EmbeddingRepo::new(pool.clone());
    assert_eq!(repo.backlog("test-model").await?, 1);

    let rows = repo.fetch_to_embed("test-model", 10).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, "turn");
    assert_eq!(rows[0].item_id, tid.0);

    let v = vec![0.1f32; 768];
    let written = repo.bulk_insert(&rows, "test-model", 768, &[v]).await?;
    assert_eq!(written, 1);
    assert_eq!(repo.backlog("test-model").await?, 0);

    // ON CONFLICT no-op.
    let v2 = vec![0.2f32; 768];
    let written2 = repo.bulk_insert(&rows, "test-model", 768, &[v2]).await?;
    assert_eq!(written2, 0);

    sup.shutdown().await?;
    Ok(())
}
