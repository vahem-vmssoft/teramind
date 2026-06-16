//! codifier §10: the bundled_context passed to the CodifyProvider must NOT
//! contain raw secrets — Redactor::apply runs over every text field before
//! the bundle is serialized into the LLM prompt.

use async_trait::async_trait;
use parking_lot::Mutex;
use std::sync::Arc;
use teramind_core::codify::{CodifyDecision, CodifyProvider, CodifyRequest, CodifyResult};
use teramind_core::ids::SessionId;
use teramind_core::redact::Redactor;
use teramind_db::repos::session::NewSession;
use teramind_db::repos::{
    AgentRepo, SessionRepo, SkillCandidateRepo, SkillObservationRepo, TraceRepo, WikiRepo,
};
use teramindd::services::codify::synthesis::{synthesize_one, SynthesisDeps};
use time::OffsetDateTime;
use uuid::Uuid;

struct CapturingProvider {
    captured: Arc<Mutex<Option<String>>>,
}

#[async_trait]
impl CodifyProvider for CapturingProvider {
    fn name(&self) -> &str {
        "capturing"
    }
    async fn codify(&self, req: CodifyRequest) -> anyhow::Result<CodifyResult> {
        *self.captured.lock() = Some(req.bundled_context.clone());
        Ok(CodifyResult {
            decision: CodifyDecision::Skip {
                reason: "test-only".into(),
            },
            input_tokens: 1,
            output_tokens: 1,
        })
    }
}

const SECRET: &str = "AKIAIOSFODNN7EXAMPLE";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn redactor_strips_secrets_from_bundled_context() -> anyhow::Result<()> {
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
    for i in 0..2 {
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
        // First session gets a wiki page laced with the secret; second uses raw turns.
        if i == 0 {
            wiki.upsert(
                sid,
                "mock-model",
                &format!("# leak\n\nAWS key {SECRET} leaked here\n"),
                0,
                0,
            )
            .await?;
        } else {
            let tid = trace
                .upsert_turn_with_id(
                    teramind_core::ids::TurnId(Uuid::new_v4()),
                    sid,
                    0,
                    started,
                    Some(&format!("turn-prompt {SECRET}")),
                )
                .await?;
            trace
                .finalize_turn(
                    tid,
                    started,
                    Some(&format!("assistant {SECRET}")),
                    None,
                    None,
                    None,
                    None,
                )
                .await?;
        }
        sids.push(SessionId(sid.0));
    }

    obs_repo
        .upsert(
            "tool_chain",
            &format!("sig-with-{SECRET}"),
            &sids,
            serde_json::json!({"note": format!("ctx {}", SECRET)}),
        )
        .await?;
    let observation = obs_repo
        .find_by_sig("tool_chain", &format!("sig-with-{SECRET}"))
        .await?
        .expect("observation must exist");

    let captured = Arc::new(Mutex::new(None));
    let deps = SynthesisDeps {
        pool: pool.clone(),
        obs: obs_repo.clone(),
        cand: cand_repo.clone(),
        provider: Arc::new(CapturingProvider {
            captured: captured.clone(),
        }),
        redactor: Arc::new(Redactor::with_default_rules()),
        input_char_budget: 16_000,
        output_token_budget: 256,
        model_label: "test".into(),
    };
    let _ = synthesize_one(&deps, observation).await?;

    let bundle = captured.lock().clone().expect("provider must be called");
    assert!(
        !bundle.contains(SECRET),
        "redactor must strip AWS access key marker from bundled prompt; bundle was:\n{bundle}"
    );
    // Sanity: the redacted marker (or its replacement) is still some sign the
    // secret-region was rendered (so we know we didn't just drop the whole
    // text).
    assert!(!bundle.is_empty(), "bundle should not be empty");

    Ok(())
}
