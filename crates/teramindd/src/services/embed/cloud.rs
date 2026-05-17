//! Cloud embedding provider stub. v1.0 exposes the type + config gate;
//! actual HTTPS plumbing arrives in v1.1.

use async_trait::async_trait;
use teramind_core::embed::{DistanceMetric, EmbedError, EmbeddingProvider, ProviderKind};

pub struct CloudProvider {
    kind: ProviderKind,
    model: String,
}

impl CloudProvider {
    pub fn new(kind: ProviderKind, model: String) -> Result<Self, EmbedError> {
        if !kind.is_cloud() {
            return Err(EmbedError::Other(format!(
                "CloudProvider built with non-cloud kind {:?}",
                kind,
            )));
        }
        Ok(Self { kind, model })
    }
}

#[async_trait]
impl EmbeddingProvider for CloudProvider {
    fn kind(&self) -> ProviderKind {
        self.kind
    }
    fn model_id(&self) -> &str {
        &self.model
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
        Err(EmbedError::Unhealthy(
            "cloud providers are stubbed in v1.0; wiring lands in v1.1".into(),
        ))
    }

    async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Err(EmbedError::Other(
            "cloud providers are stubbed in v1.0; wiring lands in v1.1".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_cloud_kind() {
        let r = CloudProvider::new(ProviderKind::Ollama, "x".into());
        assert!(r.is_err());
    }

    #[test]
    fn accepts_cloud_kind() {
        let r = CloudProvider::new(ProviderKind::Anthropic, "voyage-3".into());
        assert!(r.is_ok());
    }

    #[tokio::test]
    async fn health_check_returns_unhealthy() {
        let p = CloudProvider::new(ProviderKind::Anthropic, "voyage-3".into()).unwrap();
        assert!(p.health_check().await.is_err());
    }
}
