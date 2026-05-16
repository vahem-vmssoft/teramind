//! Summary provider trait + shared types. Lives in `teramind-core` so
//! the MCP / eval / CLI crates can depend on it without pulling in the daemon.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use crate::embed::ProviderKind;

#[derive(Debug, thiserror::Error)]
pub enum SummaryError {
    #[error("provider unhealthy: {0}")]
    Unhealthy(String),
    #[error("budget exceeded: {0}")]
    BudgetExceeded(String),
    #[error("model not found: {0}")]
    ModelNotFound(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("provider error: {0}")]
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryResult {
    pub content: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[async_trait]
pub trait SummaryProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;
    fn model_id(&self) -> &str;
    fn max_input_tokens(&self) -> usize;
    fn max_output_tokens(&self) -> usize;
    async fn health_check(&self) -> Result<(), SummaryError>;
    async fn summarize(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        max_output_tokens: usize,
    ) -> Result<SummaryResult, SummaryError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_error_classifies() {
        assert!(matches!(SummaryError::Unhealthy("x".into()), SummaryError::Unhealthy(_)));
        assert!(matches!(SummaryError::Network("x".into()), SummaryError::Network(_)));
    }

    #[test]
    fn summary_result_roundtrips_through_json() {
        let r = SummaryResult { content: "ok".into(), input_tokens: 10, output_tokens: 20 };
        let j = serde_json::to_string(&r).unwrap();
        let back: SummaryResult = serde_json::from_str(&j).unwrap();
        assert_eq!(r.input_tokens, back.input_tokens);
        assert_eq!(r.content, back.content);
    }
}
