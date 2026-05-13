use teramind_core::types::SearchRequest;
use teramind_db::{pg_supervisor::PgSupervisor, pool::DbPool, migrate};
use teramind_db::repos::{AgentRepo, SessionRepo, TraceRepo, SearchRepo};
use teramindd::services::search;
use tempfile::tempdir;

#[tokio::test]
async fn do_search_finds_seeded_turn_via_fts() {
    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().join("pg"), "teramind_test").await.unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();

    let agents = AgentRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await.unwrap();
    let sessions = SessionRepo::new(pool.clone());
    let now = time::OffsetDateTime::now_utc();
    let sid = sessions.insert(teramind_db::repos::session::NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/w", project_id: None,
        parent_session_id: None, git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u", started_at: now,
    }).await.unwrap();
    let trace = TraceRepo::new(pool.clone());
    let turn = trace.upsert_turn(sid, 0, now, Some("how to debug postgres replication lag")).await.unwrap();
    trace.finalize_turn(turn, now, Some("the replication lag means the standby is behind"), None, None, None, None).await.unwrap();
    sqlx::query("REFRESH MATERIALIZED VIEW traces_fts").execute(pool.pg()).await.unwrap();

    let repo = SearchRepo::new(pool.clone());
    let req = SearchRequest { query: "replication lag".into(), limit: 10 };
    let out = search::do_search(&repo, &req).await.unwrap();
    assert!(out.hits.len() >= 1);
    assert!(!out.degraded);

    sup.shutdown().await.unwrap();
}
