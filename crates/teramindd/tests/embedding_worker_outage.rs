//! pgvector §10: embedding worker pauses on provider outage (no rows written,
//! errors counter increments), and resumes once health returns.

use async_trait::async_trait;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use teramind_core::embed::{DistanceMetric, EmbedError, EmbeddingProvider, ProviderKind};
use teramind_core::ids::TurnId;
use teramind_core::redact::Redactor;
use teramind_db::repos::session::NewSession;
use teramind_db::repos::{AgentRepo, EmbeddingRepo, SessionRepo, TraceRepo};
use teramindd::services::embedding_worker::{EmbeddingWorker, EmbeddingWorkerDeps};
use time::OffsetDateTime;

struct FlakyEmbed {
    health_calls: AtomicU32,
    unhealthy_count: u32,
    dim: usize,
}

#[async_trait]
impl EmbeddingProvider for FlakyEmbed {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Fastembed
    }
    fn model_id(&self) -> &str {
        "mock-flaky"
    }
    fn dimension(&self) -> usize {
        self.dim
    }
    fn max_tokens(&self) -> usize {
        8192
    }
    fn distance_metric(&self) -> DistanceMetric {
        DistanceMetric::Cosine
    }
    async fn health_check(&self) -> Result<(), EmbedError> {
        let n = self.health_calls.fetch_add(1, Ordering::Relaxed);
        if n < self.unhealthy_count {
            Err(EmbedError::Unhealthy(format!("attempt {n}")))
        } else {
            Ok(())
        }
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0f32; self.dim];
                v[0] = t.chars().count() as f32;
                v
            })
            .collect())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn worker_backs_off_on_outage_then_recovers() -> anyhow::Result<()> {
    let pool = teramind_db::testing::fresh_pool().await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await?;
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
            started_at: OffsetDateTime::now_utc(),
            user_id: None,
            device_id: None,
        })
        .await?;
    let tid = trace
        .upsert_turn_with_id(
            TurnId(uuid::Uuid::new_v4()),
            sid,
            0,
            OffsetDateTime::now_utc(),
            Some("first prompt"),
        )
        .await?;
    trace
        .finalize_turn(
            tid,
            OffsetDateTime::now_utc(),
            Some("a"),
            None,
            None,
            None,
            None,
        )
        .await?;

    let repo = EmbeddingRepo::new(pool.clone());
    assert!(repo.backlog("mock:mock-flaky").await? >= 1);

    let worker = EmbeddingWorker::spawn(EmbeddingWorkerDeps {
        repo: repo.clone(),
        provider: Arc::new(FlakyEmbed {
            health_calls: AtomicU32::new(0),
            unhealthy_count: 2,
            dim: 768,
        }),
        redactor: Arc::new(Redactor::with_default_rules()),
        model: "mock:mock-flaky".into(),
        poll_interval: Duration::from_millis(150),
        batch_size: 32,
    });

    // While unhealthy: backlog stays, errors increment.
    let mut saw_error = false;
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if worker.stats.errors.load(Ordering::Relaxed) > 0 {
            saw_error = true;
            assert_eq!(
                worker.stats.written.load(Ordering::Relaxed),
                0,
                "no rows should be written during outage"
            );
            assert!(
                worker
                    .stats
                    .provider_unhealthy_since_unix
                    .load(Ordering::Relaxed)
                    > 0
            );
            break;
        }
    }
    assert!(
        saw_error,
        "errors must increment while provider is unhealthy"
    );

    // Wait for backlog to drain.
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if repo.backlog("mock:mock-flaky").await? == 0 {
            break;
        }
    }
    assert_eq!(
        repo.backlog("mock:mock-flaky").await?,
        0,
        "backlog must drain after recovery"
    );
    assert!(
        worker.stats.written.load(Ordering::Relaxed) >= 1,
        "at least one embedding row must be written after recovery"
    );

    Ok(())
}
