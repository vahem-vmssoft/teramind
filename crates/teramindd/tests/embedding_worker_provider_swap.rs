//! pgvector §10: swapping the embedding provider/model does NOT migrate rows;
//! the worker re-embeds for the new (item_kind, item_id, model) keys and the
//! UNIQUE constraint is honoured.

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::embed::{DistanceMetric, EmbedError, EmbeddingProvider, ProviderKind};
use teramind_core::ids::TurnId;
use teramind_core::redact::Redactor;
use teramind_db::repos::session::NewSession;
use teramind_db::repos::{AgentRepo, EmbeddingRepo, SessionRepo, TraceRepo};
use teramindd::services::embedding_worker::{EmbeddingWorker, EmbeddingWorkerDeps};
use time::OffsetDateTime;

struct ConstMock {
    dim: usize,
    model: &'static str,
}

#[async_trait]
impl EmbeddingProvider for ConstMock {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Fastembed
    }
    fn model_id(&self) -> &str {
        self.model
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
        Ok(())
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
async fn provider_swap_reembeds_without_touching_old_rows() -> anyhow::Result<()> {
    let pool = teramind_db::testing::fresh_pool().await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await?;
    let started = OffsetDateTime::now_utc();
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
    let mut tids = vec![];
    for i in 0..2 {
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
            .finalize_turn(tid, started, Some("a"), None, None, None, None)
            .await?;
        tids.push(tid.0);
    }

    let repo = EmbeddingRepo::new(pool.clone());

    // Embed with model A.
    let worker_a = EmbeddingWorker::spawn(EmbeddingWorkerDeps {
        repo: repo.clone(),
        provider: Arc::new(ConstMock {
            dim: 768,
            model: "modelA",
        }),
        redactor: Arc::new(Redactor::with_default_rules()),
        model: "modelA".into(),
        poll_interval: Duration::from_millis(100),
        batch_size: 32,
    });
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if repo.backlog("modelA").await? == 0 {
            break;
        }
    }
    assert_eq!(repo.backlog("modelA").await?, 0);
    worker_a.abort();

    let (rows_a,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM embeddings WHERE model='modelA' AND item_kind='turn'",
    )
    .fetch_one(pool.pg())
    .await?;
    assert_eq!(rows_a, 2, "modelA must have produced 2 turn rows");

    // Now spawn a worker against the SAME pool with modelB.
    let _worker_b = EmbeddingWorker::spawn(EmbeddingWorkerDeps {
        repo: repo.clone(),
        provider: Arc::new(ConstMock {
            dim: 768,
            model: "modelB",
        }),
        redactor: Arc::new(Redactor::with_default_rules()),
        model: "modelB".into(),
        poll_interval: Duration::from_millis(100),
        batch_size: 32,
    });
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if repo.backlog("modelB").await? == 0 {
            break;
        }
    }

    // (a) Original modelA rows still exist.
    let (rows_a_after,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM embeddings WHERE model='modelA'")
            .fetch_one(pool.pg())
            .await?;
    assert_eq!(
        rows_a_after, 2,
        "modelA rows must be untouched by the swap"
    );

    // (b) New modelB rows exist keyed by the same item_ids.
    let (rows_b,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM embeddings WHERE model='modelB' AND item_kind='turn'",
    )
    .fetch_one(pool.pg())
    .await?;
    assert_eq!(rows_b, 2, "modelB must have produced rows for the same turns");

    let (shared,): (i64,) = sqlx::query_as(
        r#"SELECT count(*) FROM embeddings a JOIN embeddings b
           ON a.item_kind = b.item_kind AND a.item_id = b.item_id
           WHERE a.model = 'modelA' AND b.model = 'modelB'"#,
    )
    .fetch_one(pool.pg())
    .await?;
    assert_eq!(shared, 2, "every turn must have one row per model");

    // (c) UNIQUE(item_kind, item_id, model) honoured: total rows = 4 for two turns.
    let (total,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM embeddings WHERE item_kind='turn'")
            .fetch_one(pool.pg())
            .await?;
    assert_eq!(total, 4);

    Ok(())
}
