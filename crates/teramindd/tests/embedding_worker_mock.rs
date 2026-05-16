//! L3: mock embedding provider feeds a real PG via the real worker.

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::embed::{DistanceMetric, EmbedError, EmbeddingProvider, ProviderKind};
use teramind_core::ids::TurnId;
use teramind_core::redact::Redactor;
use teramind_db::repos::{AgentRepo, EmbeddingRepo, SessionRepo, TraceRepo};
use teramind_db::repos::session::NewSession;
use teramindd::services::embedding_worker::{EmbeddingWorker, EmbeddingWorkerDeps};
use time::OffsetDateTime;

struct DeterministicMock { dim: usize }

#[async_trait]
impl EmbeddingProvider for DeterministicMock {
    fn kind(&self) -> ProviderKind { ProviderKind::Fastembed }
    fn model_id(&self) -> &str { "mock-model" }
    fn dimension(&self) -> usize { self.dim }
    fn max_tokens(&self) -> usize { 8192 }
    fn distance_metric(&self) -> DistanceMetric { DistanceMetric::Cosine }
    async fn health_check(&self) -> Result<(), EmbedError> { Ok(()) }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Ok(texts.iter().map(|t| {
            let mut v = vec![0.0f32; self.dim];
            v[0] = t.chars().count() as f32;
            v
        }).collect())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn worker_fills_embeddings_within_15s() -> anyhow::Result<()> {
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
        OffsetDateTime::now_utc(), Some("first prompt"),
    ).await?;

    let repo = EmbeddingRepo::new(pool.clone());
    assert_eq!(repo.backlog("mock:mock-model").await?, 1);

    let _worker = EmbeddingWorker::spawn(EmbeddingWorkerDeps {
        repo: repo.clone(),
        provider: Arc::new(DeterministicMock { dim: 768 }),
        redactor: Arc::new(Redactor::with_default_rules()),
        model: "mock:mock-model".into(),
        poll_interval: Duration::from_millis(200),
        batch_size: 32,
    });

    for _ in 0..75 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if repo.backlog("mock:mock-model").await? == 0 { break; }
    }
    assert_eq!(repo.backlog("mock:mock-model").await?, 0);

    sup.shutdown().await?;
    Ok(())
}
