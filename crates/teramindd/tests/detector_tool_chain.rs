//! Detector A: 5 sessions with identical Bash→Edit→Bash chains produce one
//! observation with frequency=5.

use teramind_core::ids::TurnId;
use teramind_db::repos::session::NewSession;
use teramind_db::repos::{AgentRepo, SessionRepo, SkillObservationRepo, TraceRepo};
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramindd::services::codify::detectors::tool_chain;
use time::OffsetDateTime;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn five_identical_chains_produce_one_observation() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await?;
    let started = OffsetDateTime::now_utc();

    for _ in 0..5 {
        let sid = sessions
            .insert(NewSession {
                agent_id: agent.id,
                agent_session_id: None,
                cwd: "/proj",
                project_id: None,
                parent_session_id: None,
                git_head: None,
                git_branch: None,
                os: "linux",
                hostname: "h",
                user_login: "u",
                started_at: started,
                user_id: None,
                device_id: None,
            })
            .await?;
        let tid = trace
            .upsert_turn_with_id(TurnId(Uuid::new_v4()), sid, 0, started, Some("build it"))
            .await?;
        trace
            .finalize_turn(tid, started, Some("done"), None, Some("claude"), None, None)
            .await?;
        // Three tool calls: cargo build, edit Cargo.toml, cargo test.
        let tc0 = trace
            .insert_tool_call_start(
                tid,
                0,
                "Bash",
                &serde_json::json!({"command":"cargo build"}),
                started,
            )
            .await?;
        trace.finalize_tool_call(tc0, "ok", false, 100).await?;
        let tc1 = trace
            .insert_tool_call_start(
                tid,
                1,
                "Edit",
                &serde_json::json!({"file_path":"Cargo.toml"}),
                started,
            )
            .await?;
        trace.finalize_tool_call(tc1, "ok", false, 50).await?;
        let tc2 = trace
            .insert_tool_call_start(
                tid,
                2,
                "Bash",
                &serde_json::json!({"command":"cargo test"}),
                started,
            )
            .await?;
        trace.finalize_tool_call(tc2, "ok", false, 100).await?;
    }

    let obs_repo = SkillObservationRepo::new(pool.clone());
    tool_chain::run(&pool, &obs_repo, time::Duration::days(30), None).await?;

    let above = obs_repo.list_open(3, 10).await?;
    assert_eq!(above.len(), 1, "exactly one observation above threshold");
    assert_eq!(above[0].frequency, 5);
    assert_eq!(above[0].kind, "tool_chain");

    sup.shutdown().await?;
    Ok(())
}
