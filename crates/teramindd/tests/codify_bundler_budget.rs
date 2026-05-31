//! codifier §10: the synthesis bundler caps total output at input_char_budget
//! and degrades by dropping later sessions/turns while preserving the wiki
//! excerpt of the first session(s) (signal-dense content).

use async_trait::async_trait;
use parking_lot::Mutex;
use std::sync::Arc;
use teramind_core::codify::{CodifyDecision, CodifyProvider, CodifyRequest, CodifyResult};
use teramind_core::ids::{SessionId, TurnId};
use teramind_core::redact::Redactor;
use teramind_db::repos::session::NewSession;
use teramind_db::repos::{
    AgentRepo, SessionRepo, SkillCandidateRepo, SkillObservationRepo, TraceRepo, WikiRepo,
};
use teramindd::services::codify::synthesis::{synthesize_one, SynthesisDeps};
use time::OffsetDateTime;
use uuid::Uuid;

struct Capture {
    captured: Arc<Mutex<Option<String>>>,
}
#[async_trait]
impl CodifyProvider for Capture {
    fn name(&self) -> &str {
        "capture"
    }
    async fn codify(&self, req: CodifyRequest) -> anyhow::Result<CodifyResult> {
        *self.captured.lock() = Some(req.bundled_context.clone());
        Ok(CodifyResult {
            decision: CodifyDecision::Skip {
                reason: "test".into(),
            },
            input_tokens: 1,
            output_tokens: 1,
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bundler_respects_char_budget_and_prefers_wiki() -> anyhow::Result<()> {
    let pool = teramind_db::testing::fresh_pool().await?;
    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let wiki = WikiRepo::new(pool.clone());
    let obs_repo = SkillObservationRepo::new(pool.clone());
    let cand_repo = SkillCandidateRepo::new(pool.clone());

    let agent = agents.upsert("claude_code", None).await?;
    let started = OffsetDateTime::now_utc();
    let mut sids = vec![];
    // Session 0: large wiki excerpt → highest-signal content.
    let s0 = sessions
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
            started_at: started,
            user_id: None,
            device_id: None,
        })
        .await?;
    let wiki_marker = "WIKI_MARKER_SESSION_ZERO_SIGNAL_DENSE";
    wiki.upsert(s0, "m", &format!("# wiki\n{wiki_marker} {}\n", "x".repeat(800)), 0, 0)
        .await?;
    sids.push(SessionId(s0.0));

    // Sessions 1..4: large turns → lower-signal, expected to be dropped under tight budget.
    for i in 1..5 {
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
                started_at: started,
                user_id: None,
                device_id: None,
            })
            .await?;
        let tid = trace
            .upsert_turn_with_id(
                TurnId(Uuid::new_v4()),
                sid,
                0,
                started,
                Some(&format!("TURN_MARKER_{i} {}", "y".repeat(600))),
            )
            .await?;
        trace
            .finalize_turn(tid, started, Some("ok"), None, None, None, None)
            .await?;
        sids.push(SessionId(sid.0));
    }

    obs_repo
        .upsert("tool_chain", "sig", &sids, serde_json::json!({}))
        .await?;
    let observation = obs_repo
        .find_by_sig("tool_chain", "sig")
        .await?
        .expect("obs");

    // Tight budget: should force truncation.
    let captured = Arc::new(Mutex::new(None));
    let deps = SynthesisDeps {
        pool: pool.clone(),
        obs: obs_repo.clone(),
        cand: cand_repo.clone(),
        provider: Arc::new(Capture {
            captured: captured.clone(),
        }),
        redactor: Arc::new(Redactor::with_default_rules()),
        input_char_budget: 500,
        output_token_budget: 64,
        model_label: "test".into(),
    };
    let _ = synthesize_one(&deps, observation).await?;

    let bundle = captured.lock().clone().expect("provider must be called");
    // Budget compliance.
    assert!(
        bundle.len() <= 500,
        "bundle must respect input_char_budget=500; got len={}",
        bundle.len()
    );
    // Wiki content from session 0 must be included (high-signal preserved).
    assert!(
        bundle.contains(wiki_marker),
        "wiki excerpt from session 0 must be preserved; bundle:\n{bundle}"
    );
    // Later turn sessions should NOT all fit — at least one TURN_MARKER must be dropped.
    let turn_count = (1..5)
        .filter(|i| bundle.contains(&format!("TURN_MARKER_{i}")))
        .count();
    assert!(
        turn_count < 4,
        "tight budget must drop later session turns; got turn_count={turn_count}, bundle:\n{bundle}"
    );

    Ok(())
}
