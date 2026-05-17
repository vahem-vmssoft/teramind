//! Ollama embedding provider (HTTP to localhost:11434).
//! Uses /api/embed (Ollama 0.1.40+) with batched input.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use teramind_core::embed::{DistanceMetric, EmbedError, EmbeddingProvider, ProviderKind};

#[derive(Clone)]
pub struct OllamaProvider {
    url: String,
    model: String,
    dimension: usize,
    max_tokens: usize,
    client: reqwest::Client,
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

#[derive(Deserialize)]
struct VersionResponse {
    #[allow(dead_code)]
    version: String,
}

impl OllamaProvider {
    pub fn new(
        url: String,
        model: String,
        dimension: usize,
        max_tokens: usize,
        timeout: Duration,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("reqwest client build");
        Self {
            url,
            model,
            dimension,
            max_tokens,
            client,
        }
    }
}

#[async_trait]
impl EmbeddingProvider for OllamaProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Ollama
    }
    fn model_id(&self) -> &str {
        &self.model
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
        let url = format!("{}/api/version", self.url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| EmbedError::Unhealthy(format!("GET {url}: {e}")))?;
        if !resp.status().is_success() {
            return Err(EmbedError::Unhealthy(format!(
                "ollama version returned {}",
                resp.status()
            )));
        }
        let _: VersionResponse = resp
            .json()
            .await
            .map_err(|e| EmbedError::Unhealthy(format!("decode version: {e}")))?;
        Ok(())
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        let url = format!("{}/api/embed", self.url);
        let req = EmbedRequest {
            model: &self.model,
            input: texts,
        };
        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| EmbedError::Network(format!("POST {url}: {e}")))?;
        if resp.status() == reqwest::StatusCode::PAYLOAD_TOO_LARGE {
            return Err(EmbedError::SizeExceeded(format!(
                "status 413 for batch of {}",
                texts.len()
            )));
        }
        if !resp.status().is_success() {
            return Err(EmbedError::Other(format!(
                "ollama embed returned {}",
                resp.status()
            )));
        }
        let body: EmbedResponse = resp
            .json()
            .await
            .map_err(|e| EmbedError::Other(format!("decode embed: {e}")))?;
        if body.embeddings.len() != texts.len() {
            return Err(EmbedError::Other(format!(
                "ollama returned {} vectors for {} inputs",
                body.embeddings.len(),
                texts.len(),
            )));
        }
        Ok(body.embeddings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_advertises_correct_kind_and_metric() {
        let p = OllamaProvider::new(
            "http://localhost:11434".into(),
            "nomic-embed-text-v2-moe".into(),
            768,
            8192,
            Duration::from_secs(10),
        );
        assert_eq!(p.kind(), ProviderKind::Ollama);
        assert_eq!(p.distance_metric(), DistanceMetric::Cosine);
        assert_eq!(p.dimension(), 768);
        assert_eq!(p.model_id(), "nomic-embed-text-v2-moe");
    }
}
