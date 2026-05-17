//! Embedding provider trait + shared types. Lives in `teramind-core` so
//! the eval crate can depend on it without pulling in the daemon.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistanceMetric {
    Cosine,
    Dot,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Ollama,
    Fastembed,
    Anthropic,
    Openai,
    Voyage,
}

impl ProviderKind {
    pub fn is_cloud(self) -> bool {
        matches!(
            self,
            ProviderKind::Anthropic | ProviderKind::Openai | ProviderKind::Voyage
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("provider unhealthy: {0}")]
    Unhealthy(String),
    #[error("input too large for batch: {0}")]
    SizeExceeded(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("provider error: {0}")]
    Other(String),
}

impl EmbedError {
    pub fn is_size_exceeded(&self) -> bool {
        matches!(self, EmbedError::SizeExceeded(_))
    }
}

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;
    fn model_id(&self) -> &str;
    fn dimension(&self) -> usize;
    fn max_tokens(&self) -> usize;
    fn distance_metric(&self) -> DistanceMetric;
    async fn health_check(&self) -> Result<(), EmbedError>;
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_kind_cloud_classification() {
        assert!(ProviderKind::Anthropic.is_cloud());
        assert!(ProviderKind::Openai.is_cloud());
        assert!(ProviderKind::Voyage.is_cloud());
        assert!(!ProviderKind::Ollama.is_cloud());
        assert!(!ProviderKind::Fastembed.is_cloud());
    }

    #[test]
    fn embed_error_size_exceeded_classifier() {
        let e = EmbedError::SizeExceeded("test".into());
        assert!(e.is_size_exceeded());
        let e2 = EmbedError::Network("test".into());
        assert!(!e2.is_size_exceeded());
    }

    #[test]
    fn provider_kind_serde_roundtrip() {
        let k = ProviderKind::Ollama;
        let s = serde_json::to_string(&k).unwrap();
        assert_eq!(s, "\"ollama\"");
        let back: ProviderKind = serde_json::from_str(&s).unwrap();
        assert_eq!(back, ProviderKind::Ollama);
    }
}
