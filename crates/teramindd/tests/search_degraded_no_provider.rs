use async_trait::async_trait;
use std::sync::Arc;
use teramind_core::embed::{DistanceMetric, EmbedError, EmbeddingProvider, ProviderKind};

struct AlwaysFailsProvider;

#[async_trait]
impl EmbeddingProvider for AlwaysFailsProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Fastembed
    }
    fn model_id(&self) -> &str {
        "broken"
    }
    fn dimension(&self) -> usize {
        768
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
    async fn embed(&self, _t: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Err(EmbedError::Other("simulated".into()))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn search_marks_degraded_when_provider_fails() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(dir.path().to_path_buf(), "teramind")
        .await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    teramind_db::migrate::run(&pool).await?;

    let search_repo = teramind_db::repos::SearchRepo::new(pool.clone());
    let weights = teramindd::services::search::BlendWeights {
        semantic: 0.5,
        ..teramindd::services::search::BlendWeights::default()
    };
    let req = teramind_core::types::SearchRequest {
        query: "anything".into(),
        limit: 5,
    };
    let out = teramindd::services::search::do_search(
        &search_repo,
        Some(Arc::new(AlwaysFailsProvider)),
        "ollama:broken",
        weights,
        &req,
    )
    .await?;
    assert!(out.degraded, "embedding failure should set degraded=true");
    sup.shutdown().await?;
    Ok(())
}
