//! A no-op embedding provider used in tests and fallback construction.
//! It always returns an error from `embed()`, which causes `do_search` to
//! skip semantic ranking (degraded path). That is correct for unit/integration
//! tests that do not need real vector search.

use async_trait::async_trait;
use teramind_core::embed::{DistanceMetric, EmbedError, EmbeddingProvider, ProviderKind};

pub struct NullEmbeddingProvider;

#[async_trait]
impl EmbeddingProvider for NullEmbeddingProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Ollama
    }
    fn model_id(&self) -> &str {
        "null"
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
        Err(EmbedError::Unhealthy("null provider".into()))
    }
    async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Err(EmbedError::Unhealthy("null provider".into()))
    }
}
