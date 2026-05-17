//! Detector B — 4 sessions with `cargo test FAILED` user prompts + a follow-up
//! diff produce one observation.

use teramind_core::ids::TurnId;
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, SkillObservationRepo, TraceRepo};
use teramind_db::repos::diff::NewFileDiff;
use teramind_db::repos::session::NewSession;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramindd::services::codify::detectors::problem_fix;
use time::OffsetDateTime;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn four_identical_failures_produce_one_observation() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let diffs = DiffRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await?;
    let started = OffsetDateTime::now_utc();

    for i in 0..4 {
        let sid = sessions.insert(NewSession {
            agent_id: agent.id, agent_session_id: None, cwd: "/proj",
            project_id: None, parent_session_id: None,
            git_head: None, git_branch: None,
            os: "linux", hostname: "h", user_login: "u",
            started_at: started, user_id: None, device_id: None,
        }).await?;
        let tid = trace.upsert_turn_with_id(
            TurnId(Uuid::new_v4()), sid, 0, started,
            Some(&format!("cargo test FAILED at file{i}.rs:42")),
        ).await?;
        trace.finalize_turn(tid, started, Some("Fixed."), None, Some("claude"), None, None).await?;
        diffs.insert(NewFileDiff {
            turn_id: Some(tid),
            session_id: sid,
            file_path: "src/lib.rs",
            rel_path: "src/lib.rs",
            attribution: teramind_core::types::file_diff::Attribution::Agent,
            language: Some("rust"),
            pre_excerpt: "old",
            post_excerpt: "new",
            unified_diff: "- pub fn foo() {}\n+ pub fn foo(x: i32) {}\n",
            pre_hash: [0u8; 32],
            post_hash: [1u8; 32],
            byte_size: 100,
            captured_at: started,
        }).await?;
    }

    let obs_repo = SkillObservationRepo::new(pool.clone());
    problem_fix::run(&pool, &obs_repo, time::Duration::days(30)).await?;

    let above = obs_repo.list_open(3, 10).await?;
    assert_eq!(above.len(), 1);
    assert_eq!(above[0].frequency, 4);
    assert_eq!(above[0].kind, "problem_fix");

    sup.shutdown().await?;
    Ok(())
}
