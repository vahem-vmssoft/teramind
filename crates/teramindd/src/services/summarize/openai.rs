//! OpenAI provider stub (v1.0). Refuses health and summarize calls
//! with a clear message; full implementation lands in v1.1.

use async_trait::async_trait;
use teramind_core::summarize::{ProviderKind, SummaryError, SummaryProvider, SummaryResult};

pub struct OpenaiProvider {
    model: String,
}

impl OpenaiProvider {
    pub fn new(model: String) -> Self {
        Self { model }
    }
}

#[async_trait]
impl SummaryProvider for OpenaiProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Openai
    }
    fn model_id(&self) -> &str {
        &self.model
    }
    fn max_input_tokens(&self) -> usize {
        16384
    }
    fn max_output_tokens(&self) -> usize {
        1500
    }

    async fn health_check(&self) -> Result<(), SummaryError> {
        Err(SummaryError::Unhealthy(
            "openai provider is stubbed in v1.0; wiring lands in v1.1".into(),
        ))
    }

    async fn summarize(&self, _: &str, _: &str, _: usize) -> Result<SummaryResult, SummaryError> {
        Err(SummaryError::Other(
            "openai provider is stubbed in v1.0; wiring lands in v1.1".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_check_returns_unhealthy() {
        let p = OpenaiProvider::new("gpt-4o-mini".into());
        assert!(p.health_check().await.is_err());
    }
}
