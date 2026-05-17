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

// ─── Session snapshot types ───────────────────────────────────────────────────
// Defined here so both teramind-db (WikiRepo::load_snapshot) and the daemon
// (digest builder) can share the same types without a circular dependency.

use crate::ids::{SessionId, ToolCallId, TurnId};
use crate::types::file_diff::Attribution;
use serde_json::Value;
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRow {
    pub id: TurnId,
    pub ordinal: i32,
    pub user_prompt: Option<String>,
    pub assistant_text: Option<String>,
    pub thinking: Option<String>,
    pub started_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRow {
    pub id: ToolCallId,
    pub turn_id: TurnId,
    pub name: String,
    pub input: Value,
    pub output: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiffRow {
    pub turn_id: Option<TurnId>,
    pub rel_path: String,
    pub language: Option<String>,
    pub attribution: Attribution,
    pub unified_diff: String,
    pub pre_excerpt: String,
    pub post_excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub session_id: SessionId,
    pub cwd: String,
    pub started_at: OffsetDateTime,
    pub ended_at: OffsetDateTime,
    pub end_reason: String,
    pub git_branch: Option<String>,
    pub git_head: Option<String>,
    pub turns: Vec<TurnRow>,
    pub tool_calls: Vec<ToolCallRow>,
    pub file_diffs: Vec<FileDiffRow>,
}

impl SessionSnapshot {
    pub fn turn_count(&self) -> usize {
        self.turns.len()
    }
    pub fn duration_secs(&self) -> i64 {
        (self.ended_at - self.started_at).whole_seconds()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_error_classifies() {
        assert!(matches!(
            SummaryError::Unhealthy("x".into()),
            SummaryError::Unhealthy(_)
        ));
        assert!(matches!(
            SummaryError::Network("x".into()),
            SummaryError::Network(_)
        ));
    }

    #[test]
    fn summary_result_roundtrips_through_json() {
        let r = SummaryResult {
            content: "ok".into(),
            input_tokens: 10,
            output_tokens: 20,
        };
        let j = serde_json::to_string(&r).unwrap();
        let back: SummaryResult = serde_json::from_str(&j).unwrap();
        assert_eq!(r.input_tokens, back.input_tokens);
        assert_eq!(r.content, back.content);
    }
}
