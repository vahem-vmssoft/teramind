//! Detector C — calls a CodifyProvider once per cycle. With NullProvider it
//! always returns Skip, so no observation is emitted, but the call path
//! works end-to-end without panicking.

use std::sync::Arc;
use teramind_core::ids::TurnId;
use teramind_db::repos::session::NewSession;
use teramind_db::repos::{AgentRepo, SessionRepo, SkillObservationRepo, TraceRepo};
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramindd::services::codify::detectors::llm_proposal;
use teramindd::services::codify::null::NullCodifyProvider;
use time::OffsetDateTime;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn null_provider_yields_no_observation() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let started = OffsetDateTime::now_utc();
    let agent = agents.upsert("claude_code", None).await?;
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
        sessions.end(sid, started, "stop_hook").await?;
        let tid = trace
            .upsert_turn_with_id(TurnId(Uuid::new_v4()), sid, 0, started, Some("x"))
            .await?;
        trace
            .finalize_turn(tid, started, Some("y"), None, None, None, None)
            .await?;
    }

    let obs = SkillObservationRepo::new(pool.clone());
    let provider: Arc<dyn teramind_core::codify::CodifyProvider> = Arc::new(NullCodifyProvider);
    llm_proposal::run(&pool, &obs, provider.as_ref(), None).await?;

    assert!(obs
        .list_recent(Some("llm_proposal"), None, 10)
        .await?
        .is_empty());

    sup.shutdown().await?;
    Ok(())
}
