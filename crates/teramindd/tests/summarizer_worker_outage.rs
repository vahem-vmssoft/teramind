//! summarizer §10: provider unhealthy → worker pauses without exiting,
//! resumes when provider becomes healthy. SummarizerStats records the
//! unhealthy window.

use async_trait::async_trait;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use teramind_core::ids::TurnId;
use teramind_core::redact::Redactor;
use teramind_core::summarize::{ProviderKind, SummaryError, SummaryProvider, SummaryResult};
use teramind_db::repos::session::NewSession;
use teramind_db::repos::{AgentRepo, SessionRepo, TraceRepo, WikiRepo};
use teramindd::services::summarizer_worker::{SummarizerDeps, SummarizerWorker};
use time::OffsetDateTime;

struct FlakyProvider {
    health_calls: AtomicU32,
    unhealthy_count: u32,
}

#[async_trait]
impl SummaryProvider for FlakyProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Ollama
    }
    fn model_id(&self) -> &str {
        "mock:flaky"
    }
    fn max_input_tokens(&self) -> usize {
        16384
    }
    fn max_output_tokens(&self) -> usize {
        1500
    }
    async fn health_check(&self) -> Result<(), SummaryError> {
        let n = self.health_calls.fetch_add(1, Ordering::Relaxed);
        if n < self.unhealthy_count {
            Err(SummaryError::Unhealthy(format!("attempt {n}")))
        } else {
            Ok(())
        }
    }
    async fn summarize(
        &self,
        _system: &str,
        _user: &str,
        _max_output_tokens: usize,
    ) -> Result<SummaryResult, SummaryError> {
        Ok(SummaryResult {
            content: "# Summary\n\nrecovered\n".into(),
            input_tokens: 10,
            output_tokens: 20,
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pauses_on_outage_then_resumes() -> anyhow::Result<()> {
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

    let provider = Arc::new(FlakyProvider {
        health_calls: AtomicU32::new(0),
        unhealthy_count: 2,
    });

    let worker = SummarizerWorker::spawn(SummarizerDeps {
        repo: wiki.clone(),
        provider: provider.clone(),
        redactor: Arc::new(Redactor::with_default_rules()),
        model: "mock:flaky".into(),
        poll_interval: Duration::from_millis(150),
        min_turns: 3,
        min_duration_secs: 60,
        input_char_budget: 8000,
        output_token_budget: 1500,
    });

    // While unhealthy, observe stats.provider_unhealthy_since_unix > 0.
    let mut saw_unhealthy_window = false;
    for _ in 0..15 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if worker
            .stats
            .provider_unhealthy_since_unix
            .load(Ordering::Relaxed)
            > 0
        {
            saw_unhealthy_window = true;
            assert!(
                wiki.get_for_session(sid, "mock:flaky").await?.is_none(),
                "no wiki should be written during outage"
            );
            break;
        }
    }
    assert!(
        saw_unhealthy_window,
        "must record unhealthy_since while provider is unhealthy"
    );

    // Wait for recovery: wiki row appears.
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if wiki.get_for_session(sid, "mock:flaky").await?.is_some() {
            break;
        }
    }
    let page = wiki
        .get_for_session(sid, "mock:flaky")
        .await?
        .expect("wiki must exist after recovery");
    assert!(page.content.contains("recovered"));
    // After recovery the unhealthy_since is reset to 0.
    assert_eq!(
        worker
            .stats
            .provider_unhealthy_since_unix
            .load(Ordering::Relaxed),
        0,
        "unhealthy_since must clear once healthy"
    );

    Ok(())
}
