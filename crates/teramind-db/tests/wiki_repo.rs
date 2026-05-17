use teramind_db::repos::session::NewSession;
use teramind_db::repos::{AgentRepo, SessionRepo, WikiRepo};
use time::OffsetDateTime;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wiki_repo_backlog_and_upsert() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(dir.path().to_path_buf(), "teramind")
        .await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    teramind_db::migrate::run(&pool).await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let wiki = WikiRepo::new(pool.clone());

    let agent = agents.upsert("claude_code", None).await?;
    let sid = sessions
        .insert(NewSession {
            agent_id: agent.id,
            agent_session_id: None,
            cwd: "/p",
            project_id: None,
            parent_session_id: None,
            git_head: None,
            git_branch: None,
            os: "linux",
            hostname: "h",
            user_login: "u",
            started_at: OffsetDateTime::now_utc(),
            user_id: None,
            device_id: None,
        })
        .await?;

    // Session not ended yet -> backlog 0.
    assert_eq!(wiki.backlog("ollama:test").await?, 0);

    sessions
        .end(sid, OffsetDateTime::now_utc(), "stop_hook")
        .await?;

    // Now backlog == 1.
    assert_eq!(wiki.backlog("ollama:test").await?, 1);
    let candidates = wiki.fetch_sessions_to_summarize("ollama:test", 10).await?;
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].session_id, sid);

    // Upsert a page.
    wiki.upsert(sid, "ollama:test", "# Summary\nhi", 10, 20)
        .await?;
    assert_eq!(wiki.backlog("ollama:test").await?, 0);

    let got = wiki.get_for_session(sid, "ollama:test").await?;
    assert!(got.is_some());
    assert_eq!(got.unwrap().content, "# Summary\nhi");

    // latest_for_cwd
    let latest = wiki.latest_for_cwd("/p").await?;
    assert!(latest.is_some());

    // Re-upsert with new content (overwrites).
    wiki.upsert(sid, "ollama:test", "# Summary\nv2", 11, 21)
        .await?;
    let got = wiki.get_for_session(sid, "ollama:test").await?.unwrap();
    assert_eq!(got.content, "# Summary\nv2");

    // Skip marker: empty content -> latest_for_cwd should exclude it.
    let sid2 = sessions
        .insert(NewSession {
            agent_id: agent.id,
            agent_session_id: None,
            cwd: "/q",
            project_id: None,
            parent_session_id: None,
            git_head: None,
            git_branch: None,
            os: "linux",
            hostname: "h",
            user_login: "u",
            started_at: OffsetDateTime::now_utc(),
            user_id: None,
            device_id: None,
        })
        .await?;
    sessions
        .end(sid2, OffsetDateTime::now_utc(), "stop_hook")
        .await?;
    wiki.mark_skipped(sid2, "ollama:test").await?;
    assert!(
        wiki.latest_for_cwd("/q").await?.is_none(),
        "skipped sessions must not show up in latest_for_cwd"
    );

    sup.shutdown().await?;
    Ok(())
}
