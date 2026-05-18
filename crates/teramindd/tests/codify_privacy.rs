//! With a session marked DeniedKeepLocal, detector A skips it.

use teramind_core::ids::TurnId;
use teramind_db::repos::session::NewSession;
use teramind_db::repos::{AgentRepo, SessionRepo, SkillObservationRepo, TraceRepo};
use teramindd::services::codify::detectors::tool_chain;
use teramindd::services::decision_cache::{DecisionCache, ShareDecision};
use time::OffsetDateTime;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn denied_sessions_excluded_from_observations() -> anyhow::Result<()> {
    let pool = teramind_db::testing::fresh_pool().await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await?;
    let started = OffsetDateTime::now_utc();

    let mut sids = vec![];
    for _ in 0..4 {
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
            .upsert_turn_with_id(TurnId(Uuid::new_v4()), sid, 0, started, Some("x"))
            .await?;
        trace
            .finalize_turn(tid, started, Some("y"), None, None, None, None)
            .await?;
        let tc0 = trace
            .insert_tool_call_start(
                tid,
                0,
                "Bash",
                &serde_json::json!({"command": "cargo build"}),
                started,
            )
            .await?;
        trace.finalize_tool_call(tc0, "ok", false, 100).await?;
        let tc1 = trace
            .insert_tool_call_start(
                tid,
                1,
                "Bash",
                &serde_json::json!({"command": "cargo test"}),
                started,
            )
            .await?;
        trace.finalize_tool_call(tc1, "ok", false, 100).await?;
        sids.push(sid);
    }

    // Mark the first session as DeniedKeepLocal.
    let cache = DecisionCache::new();
    cache.set_initial(sids[0], ShareDecision::DeniedKeepLocal);

    let obs = SkillObservationRepo::new(pool.clone());
    tool_chain::run(&pool, &obs, time::Duration::days(30), Some(cache.clone())).await?;

    let above = obs.list_open(3, 10).await?;
    assert_eq!(above.len(), 1);
    assert_eq!(above[0].frequency, 3, "denied session must be excluded");
    let denied_uuid = sids[0].0;
    assert!(
        !above[0].session_ids.contains(&denied_uuid),
        "denied session id must not appear in observation"
    );

    Ok(())
}
