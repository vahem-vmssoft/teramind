//! summarizer §10: ModelNotFound surfaces via SummarizerStats; no wiki row is
//! written and the errors counter increments.

use async_trait::async_trait;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::ids::TurnId;
use teramind_core::redact::Redactor;
use teramind_core::summarize::{ProviderKind, SummaryError, SummaryProvider, SummaryResult};
use teramind_db::repos::session::NewSession;
use teramind_db::repos::{AgentRepo, SessionRepo, TraceRepo, WikiRepo};
use teramindd::services::summarizer_worker::{SummarizerDeps, SummarizerWorker};
use time::OffsetDateTime;

struct MissingModel;

#[async_trait]
impl SummaryProvider for MissingModel {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Ollama
    }
    fn model_id(&self) -> &str {
        "mock:missing"
    }
    fn max_input_tokens(&self) -> usize {
        16384
    }
    fn max_output_tokens(&self) -> usize {
        1500
    }
    async fn health_check(&self) -> Result<(), SummaryError> {
        Ok(())
    }
    async fn summarize(
        &self,
        _system: &str,
        _user: &str,
        _max_output_tokens: usize,
    ) -> Result<SummaryResult, SummaryError> {
        Err(SummaryError::ModelNotFound("mock:missing not pulled".into()))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn model_not_found_surfaces_error_and_skips_write() -> anyhow::Result<()> {
    let pool = teramind_db::testing::fresh_pool().await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let wiki = WikiRepo::new(pool.clone());

    let agent = agents.upsert("claude_code", None).await?;
    let started = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let ended = OffsetDateTime::from_unix_timestamp(1_700_001_000).unwrap();
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
    for i in 0..3 {
        let tid = trace
            .upsert_turn_with_id(
                TurnId(uuid::Uuid::new_v4()),
                sid,
                i,
                started,
                Some(&format!("p{i}")),
            )
            .await?;
        trace
            .finalize_turn(tid, started, Some("r"), None, Some("test"), None, None)
            .await?;
    }
    sessions.end(sid, ended, "stop_hook").await?;

    let worker = SummarizerWorker::spawn(SummarizerDeps {
        repo: wiki.clone(),
        provider: Arc::new(MissingModel),
        redactor: Arc::new(Redactor::with_default_rules()),
        model: "mock:missing".into(),
        poll_interval: Duration::from_millis(100),
        min_turns: 3,
        min_duration_secs: 60,
        input_char_budget: 8000,
        output_token_budget: 1500,
    });

    // Wait for at least one error to be counted.
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if worker.stats.errors.load(Ordering::Relaxed) > 0 {
            break;
        }
    }
    assert!(
        worker.stats.errors.load(Ordering::Relaxed) > 0,
        "errors counter must increment on ModelNotFound"
    );
    assert_eq!(
        worker.stats.written.load(Ordering::Relaxed),
        0,
        "no wiki page should be written when the model is missing"
    );
    assert!(
        wiki.get_for_session(sid, "mock:missing").await?.is_none(),
        "wiki row must NOT exist when model is missing"
    );

    Ok(())
}
