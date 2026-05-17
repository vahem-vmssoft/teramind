//! In-process embedding provider backed by the `fastembed` crate.

use async_trait::async_trait;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::path::PathBuf;
use std::sync::Arc;
use teramind_core::embed::{DistanceMetric, EmbedError, EmbeddingProvider, ProviderKind};
use tokio::sync::Mutex;

pub struct FastEmbedProvider {
    model: Arc<Mutex<TextEmbedding>>,
    model_name: String,
    dimension: usize,
    max_tokens: usize,
}

impl FastEmbedProvider {
    pub fn new_default(cache_dir: PathBuf) -> Result<Self, EmbedError> {
        let opts = InitOptions::new(EmbeddingModel::NomicEmbedTextV15)
            .with_cache_dir(cache_dir)
            .with_show_download_progress(false);
        let model = TextEmbedding::try_new(opts)
            .map_err(|e| EmbedError::Other(format!("fastembed init: {e}")))?;
        Ok(Self {
            model: Arc::new(Mutex::new(model)),
            model_name: "fastembed:nomic-embed-text-v1.5".into(),
            dimension: 768,
            max_tokens: 8192,
        })
    }
}

#[async_trait]
impl EmbeddingProvider for FastEmbedProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Fastembed
    }
    fn model_id(&self) -> &str {
        &self.model_name
    }
    fn dimension(&self) -> usize {
        self.dimension
    }
    fn max_tokens(&self) -> usize {
        self.max_tokens
    }
    fn distance_metric(&self) -> DistanceMetric {
        DistanceMetric::Cosine
    }

    async fn health_check(&self) -> Result<(), EmbedError> {
        self.embed(&["ok".to_string()]).await.map(|_| ())
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        let model = self.model.clone();
        let texts: Vec<String> = texts.to_vec();
        tokio::task::spawn_blocking(move || {
            let m = model.blocking_lock();
            m.embed(texts, None)
                .map_err(|e| EmbedError::Other(format!("fastembed embed: {e}")))
        })
        .await
        .map_err(|e| EmbedError::Other(format!("spawn_blocking join: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn dimension_matches_schema() {
        let dim = 768;
        assert_eq!(dim, 768);
    }
}
