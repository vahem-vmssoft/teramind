//! Skill-codifier provider trait. Pure data + trait; impls live under
//! teramindd::services::codify::{ollama, anthropic, null}.

use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct CodifyRequest {
    pub observation_kind: String,
    pub bundled_context: String,
    pub frequency: u32,
    pub cwds: Vec<String>,
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct CodifyResult {
    pub decision: CodifyDecision,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone)]
pub enum CodifyDecision {
    Skip {
        reason: String,
    },
    Skill {
        name: String,
        description: String,
        body: String,
        applies_to_cwds: Vec<String>,
    },
}

#[async_trait]
pub trait CodifyProvider: Send + Sync {
    async fn codify(&self, req: CodifyRequest) -> anyhow::Result<CodifyResult>;
    fn name(&self) -> &str;
}
