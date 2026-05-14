use teramind_db::repos::{AgentRepo, DiffRepo, SearchRepo, SessionRepo};
use teramind_db::repos::diff::NewFileDiff;
use teramind_db::repos::session::NewSession;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramind_core::types::file_diff::Attribution;
use time::OffsetDateTime;

#[tokio::test]
async fn diff_excerpts_for_cwd_files_filters_by_rel_path() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let diffs = DiffRepo::new(pool.clone());
    let search = SearchRepo::new(pool.clone());

    let agent = agents.upsert("claude_code", None).await?;
    let sid = sessions.insert(NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/proj",
        project_id: None, parent_session_id: None, git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u",
        started_at: OffsetDateTime::now_utc(),
    }).await?;

    let now = OffsetDateTime::now_utc();
    diffs.insert(NewFileDiff {
        turn_id: None, session_id: sid,
        file_path: "/proj/src/foo.rs", rel_path: "src/foo.rs",
        attribution: Attribution::Agent, language: Some("rust"),
        pre_excerpt: "old foo", post_excerpt: "new foo",
        unified_diff: "--- a/src/foo.rs\n+++ b/src/foo.rs\n-old foo\n+new foo\n",
        pre_hash: [0u8;32], post_hash: [1u8;32], byte_size: 7, captured_at: now,
    }).await?;
    diffs.insert(NewFileDiff {
        turn_id: None, session_id: sid,
        file_path: "/proj/src/bar.rs", rel_path: "src/bar.rs",
        attribution: Attribution::Agent, language: Some("rust"),
        pre_excerpt: "old bar", post_excerpt: "new bar",
        unified_diff: "--- a/src/bar.rs\n+++ b/src/bar.rs\n-old bar\n+new bar\n",
        pre_hash: [2u8;32], post_hash: [3u8;32], byte_size: 7, captured_at: now,
    }).await?;

    let hits = search
        .diff_excerpts_for_cwd_files(&["src/foo.rs".to_string()], 10)
        .await?;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].rel_path, "src/foo.rs");

    sup.shutdown().await?;
    Ok(())
}
