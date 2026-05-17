//! L3: mock SummaryProvider drives a real PG via the real worker.

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::ids::TurnId;
use teramind_core::redact::Redactor;
use teramind_core::summarize::{ProviderKind, SummaryError, SummaryProvider, SummaryResult};
use teramind_db::repos::session::NewSession;
use teramind_db::repos::{AgentRepo, SessionRepo, TraceRepo, WikiRepo};
use teramindd::services::summarizer_worker::{SummarizerDeps, SummarizerWorker};
use time::OffsetDateTime;

struct EchoProvider;

#[async_trait]
impl SummaryProvider for EchoProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Ollama
    }
    fn model_id(&self) -> &str {
        "mock:echo"
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
        user: &str,
        _max_output_tokens: usize,
    ) -> Result<SummaryResult, SummaryError> {
        // Echo: produce a stable Markdown wrapping the first 100 chars of the digest.
        let preview: String = user.chars().take(100).collect();
        let content =
            format!("# Summary\n\nDigest excerpt: {preview}\n\n# Files changed\n\n- (mock)\n");
        Ok(SummaryResult {
            content,
            input_tokens: 10,
            output_tokens: 20,
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn worker_writes_wiki_for_ended_session_within_10s() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(dir.path().to_path_buf(), "teramind")
        .await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    teramind_db::migrate::run(&pool).await?;

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
    // Three turns (over min_turns=3).
    for i in 0..3 {
        let tid = trace
            .upsert_turn_with_id(
                TurnId(uuid::Uuid::new_v4()),
                sid,
                i,
                started,
                Some(&format!("prompt {i}")),
            )
            .await?;
        trace
            .finalize_turn(
                tid,
                started,
                Some(&format!("response {i}")),
                None,
                Some("test"),
                None,
                None,
            )
            .await?;
    }
    sessions.end(sid, ended, "stop_hook").await?;

    let _worker = SummarizerWorker::spawn(SummarizerDeps {
        repo: wiki.clone(),
        provider: Arc::new(EchoProvider),
        redactor: Arc::new(Redactor::with_default_rules()),
        model: "mock:echo".into(),
        poll_interval: Duration::from_millis(200),
        min_turns: 3,
        min_duration_secs: 60,
        input_char_budget: 8000,
        output_token_budget: 1500,
    });

    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if wiki.backlog("mock:echo").await? == 0 {
            break;
        }
    }
    let page = wiki
        .get_for_session(sid, "mock:echo")
        .await?
        .expect("wiki should exist");
    assert!(page.content.contains("# Summary"));
    assert!(page.content.contains("Digest excerpt"));
    assert_eq!(page.input_tokens, 10);
    assert_eq!(page.output_tokens, 20);

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn short_session_is_skipped() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(dir.path().to_path_buf(), "teramind")
        .await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    teramind_db::migrate::run(&pool).await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let wiki = WikiRepo::new(pool.clone());

    let agent = agents.upsert("claude_code", None).await?;
    let started = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let ended = OffsetDateTime::from_unix_timestamp(1_700_000_005).unwrap(); // 5 seconds
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
        .upsert_turn_with_id(TurnId(uuid::Uuid::new_v4()), sid, 0, started, Some("hi"))
        .await?;
    trace
        .finalize_turn(tid, started, Some("hello"), None, Some("test"), None, None)
        .await?;
    sessions.end(sid, ended, "stop_hook").await?;

    let _worker = SummarizerWorker::spawn(SummarizerDeps {
        repo: wiki.clone(),
        provider: Arc::new(EchoProvider),
        redactor: Arc::new(Redactor::with_default_rules()),
        model: "mock:echo".into(),
        poll_interval: Duration::from_millis(200),
        min_turns: 3,
        min_duration_secs: 60,
        input_char_budget: 8000,
        output_token_budget: 1500,
    });

    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if wiki.backlog("mock:echo").await? == 0 {
            break;
        }
    }
    let page = wiki
        .get_for_session(sid, "mock:echo")
        .await?
        .expect("sentinel skip row");
    assert_eq!(page.content, "", "short session should get a sentinel skip");

    sup.shutdown().await?;
    Ok(())
}
