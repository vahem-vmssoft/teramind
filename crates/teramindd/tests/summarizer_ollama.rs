//! L3: real Ollama (host-local, GPU-preferred). Skips when probe fails.

use std::sync::Arc;
use std::time::Duration;
use teramind_core::ids::TurnId;
use teramind_core::redact::Redactor;
use teramind_db::repos::session::NewSession;
use teramind_db::repos::{AgentRepo, SessionRepo, TraceRepo, WikiRepo};
use teramindd::config::SummarizeConfig;
use teramindd::services::summarize::build_provider;
use teramindd::services::summarizer_worker::{SummarizerDeps, SummarizerWorker};
use time::OffsetDateTime;

async fn probe_ollama() -> bool {
    reqwest::Client::new()
        .get("http://localhost:11434/api/version")
        .timeout(Duration::from_millis(500))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ollama_summarizes_session_with_section_headers() -> anyhow::Result<()> {
    if !probe_ollama().await {
        eprintln!("ollama not running on localhost:11434, skipping");
        return Ok(());
    }

    let pool = teramind_db::testing::fresh_pool().await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let wiki = WikiRepo::new(pool.clone());

    let agent = agents.upsert("claude_code", None).await?;
    let started = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let ended = OffsetDateTime::from_unix_timestamp(1_700_002_500).unwrap();
    let sid = sessions
        .insert(NewSession {
            agent_id: agent.id,
            agent_session_id: None,
            cwd: "/openvms-port",
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
    for i in 0..5 {
        let tid = trace
            .upsert_turn_with_id(
                TurnId(uuid::Uuid::new_v4()),
                sid,
                i,
                started,
                Some(&format!(
                    "Port the configure.ac autoconf check #{i} for OpenVMS x86"
                )),
            )
            .await?;
        trace
            .finalize_turn(
                tid,
                started,
                Some(&format!(
                    "Replaced AC_CHECK_FUNC([fork]) with vfork-aware probe #{i}"
                )),
                None,
                Some("test"),
                None,
                None,
            )
            .await?;
    }
    sessions.end(sid, ended, "stop_hook").await?;

    // Use a lighter model than the production default (qwen3.6:latest) so the
    // test doesn't peg memory + GPU on consumer hardware. gemma4:26b is a
    // sensible middle ground: substantial enough to produce real summaries
    // with section headers, light enough to finish in <3 min on M-series GPUs.
    //
    // gemma4:26b carries a "thinking" preamble that consumes a few hundred
    // tokens before visible content; the production default of 1500 isn't
    // enough room for both reasoning and a 4-section structured summary.
    // Bump to 4000 just for this test.
    let cfg = SummarizeConfig {
        model: "gemma4:26b".into(),
        output_token_budget: 4000,
        ..SummarizeConfig::default()
    };
    let secrets = tempfile::tempdir()?.path().join("secrets.toml");
    let provider = build_provider(&cfg, &secrets)?;

    let _worker = SummarizerWorker::spawn(SummarizerDeps {
        repo: wiki.clone(),
        provider: provider.clone(),
        redactor: Arc::new(Redactor::with_default_rules()),
        model: format!("ollama:{}", cfg.model),
        poll_interval: Duration::from_secs(1),
        min_turns: 3,
        min_duration_secs: 60,
        input_char_budget: cfg.input_char_budget,
        output_token_budget: cfg.output_token_budget,
    });

    // Allow up to 180s wall clock for Ollama+gemma4:26b to summarize.
    for _ in 0..180 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if wiki.backlog(&format!("ollama:{}", cfg.model)).await? == 0 {
            break;
        }
    }
    let page = wiki
        .get_for_session(sid, &format!("ollama:{}", cfg.model))
        .await?
        .expect("wiki must exist after worker drains");
    assert!(!page.content.is_empty(), "non-empty summary expected");

    let required_headers = [
        "# Summary",
        "# Files changed",
        "# Decisions & gotchas",
        "# Follow-ups",
    ];
    let missing: Vec<_> = required_headers
        .iter()
        .filter(|h| !page.content.contains(*h))
        .collect();
    // Allow a single missing header (chat models occasionally rename them);
    // assert at least 3 of 4 are present.
    assert!(
        missing.len() <= 1,
        "expected at least 3/4 spec section headers; missing: {:?}\ncontent:\n{}",
        missing,
        page.content
    );

    Ok(())
}
