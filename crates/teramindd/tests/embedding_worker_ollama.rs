//! L3: real Ollama (host-local, GPU-preferred). Skips when probe fails.

use std::sync::Arc;
use std::time::Duration;
use teramind_core::ids::TurnId;
use teramind_db::repos::{AgentRepo, EmbeddingRepo, SearchRepo, SessionRepo, TraceRepo};
use teramind_db::repos::session::NewSession;
use teramindd::config::EmbedConfig;
use teramindd::services::embed::build_provider;
use teramindd::services::embedding_worker::{EmbeddingWorker, EmbeddingWorkerDeps};
use time::OffsetDateTime;

async fn probe_ollama() -> bool {
    reqwest::Client::new()
        .get("http://localhost:11434/api/version")
        .timeout(Duration::from_millis(500))
        .send().await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ollama_e2e_paraphrase_lookup() -> anyhow::Result<()> {
    if !probe_ollama().await {
        eprintln!("ollama not running on localhost:11434, skipping");
        return Ok(());
    }

    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(
        dir.path().to_path_buf(), "teramind",
    ).await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    teramind_db::migrate::run(&pool).await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await?;
    let sid = sessions.insert(NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/p",
        project_id: None, parent_session_id: None,
        git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u",
        started_at: OffsetDateTime::now_utc(),
    }).await?;
    let _ = trace.upsert_turn_with_id(
        TurnId(uuid::Uuid::new_v4()), sid, 0,
        OffsetDateTime::now_utc(),
        Some("the access credential cycle is renewed before timeout"),
    ).await?;

    let cfg = EmbedConfig::default();
    let provider = build_provider(&cfg)?;
    let model = format!("ollama:{}", cfg.model);

    let repo = EmbeddingRepo::new(pool.clone());
    let _worker = EmbeddingWorker::spawn(EmbeddingWorkerDeps {
        repo: repo.clone(),
        provider: provider.clone(),
        redactor: Arc::new(teramind_core::redact::Redactor::with_default_rules()),
        model: model.clone(),
        poll_interval: Duration::from_millis(500),
        batch_size: 8,
    });
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if repo.backlog(&model).await? == 0 { break; }
    }
    assert_eq!(repo.backlog(&model).await?, 0, "worker should drain backlog");

    let q_emb = provider.embed(&[
        "how does the JWT refresh flow work".to_string()
    ]).await
        .map_err(|e| anyhow::anyhow!("embed query: {e}"))?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("no embedding returned"))?;

    let search = SearchRepo::new(pool.clone());
    let hits = search.vector_search_turns(&q_emb, &model, 5).await?;
    assert!(!hits.is_empty(), "semantic search should return the paraphrased turn");
    assert!(hits[0].semantic_score > 0.3,
        "expected non-trivial cosine similarity, got {}", hits[0].semantic_score);

    sup.shutdown().await?;
    Ok(())
}
