//! Test-only null SummaryProvider: returns empty Markdown immediately.

use async_trait::async_trait;
use teramind_core::summarize::*;

pub struct NullSummaryProvider;

#[async_trait]
impl SummaryProvider for NullSummaryProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Ollama
    }
    fn model_id(&self) -> &str {
        "test:null"
    }
    fn max_input_tokens(&self) -> usize {
        16384
    }
    fn max_output_tokens(&self) -> usize {
        1500
    }
    async fn health_check(&self) -> Result<(), SummaryError> {
        Ok(())
    }
    async fn summarize(&self, _: &str, _: &str, _: usize) -> Result<SummaryResult, SummaryError> {
        Ok(SummaryResult {
            content: String::new(),
            input_tokens: 0,
            output_tokens: 0,
        })
    }
}
