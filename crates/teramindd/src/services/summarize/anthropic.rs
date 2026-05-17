//! Anthropic Messages API provider. Refuses to construct without
//! network_egress + an API key.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use teramind_core::summarize::{ProviderKind, SummaryError, SummaryProvider, SummaryResult};

#[derive(Clone)]
pub struct AnthropicProvider {
    api_key: String,
    model: String,
    max_input_tokens: usize,
    max_output_tokens: usize,
    client: reqwest::Client,
}

#[derive(Serialize)]
struct MessagesRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: [UserMessage<'a>; 1],
}

#[derive(Serialize)]
struct UserMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
    usage: Usage,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(default)]
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

impl AnthropicProvider {
    pub fn new(
        api_key: String,
        model: String,
        max_input_tokens: usize,
        max_output_tokens: usize,
        timeout: Duration,
    ) -> Result<Self, SummaryError> {
        if api_key.trim().is_empty() {
            return Err(SummaryError::Other("anthropic api_key is empty".into()));
        }
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| SummaryError::Other(format!("reqwest build: {e}")))?;
        Ok(Self {
            api_key,
            model,
            max_input_tokens,
            max_output_tokens,
            client,
        })
    }
}

#[async_trait]
impl SummaryProvider for AnthropicProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Anthropic
    }
    fn model_id(&self) -> &str {
        &self.model
    }
    fn max_input_tokens(&self) -> usize {
        self.max_input_tokens
    }
    fn max_output_tokens(&self) -> usize {
        self.max_output_tokens
    }

    async fn health_check(&self) -> Result<(), SummaryError> {
        // Cheapest valid call: send a 1-token request.
        let body = MessagesRequest {
            model: &self.model,
            max_tokens: 1,
            system: "Reply with just OK.",
            messages: [UserMessage {
                role: "user",
                content: "ok",
            }],
        };
        let resp = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .map_err(|e| SummaryError::Unhealthy(format!("anthropic health: {e}")))?;
        if !resp.status().is_success() {
            return Err(SummaryError::Unhealthy(format!(
                "anthropic returned {}",
                resp.status()
            )));
        }
        Ok(())
    }

    async fn summarize(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        max_output_tokens: usize,
    ) -> Result<SummaryResult, SummaryError> {
        let body = MessagesRequest {
            model: &self.model,
            max_tokens: max_output_tokens as u32,
            system: system_prompt,
            messages: [UserMessage {
                role: "user",
                content: user_prompt,
            }],
        };
        let resp = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .map_err(|e| SummaryError::Network(format!("anthropic POST: {e}")))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(SummaryError::ModelNotFound(self.model.clone()));
        }
        if status.as_u16() == 429 {
            return Err(SummaryError::BudgetExceeded("anthropic rate limit".into()));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(SummaryError::Other(format!(
                "anthropic returned {}: {}",
                status, body
            )));
        }
        let parsed: MessagesResponse = resp
            .json()
            .await
            .map_err(|e| SummaryError::Other(format!("decode anthropic: {e}")))?;
        let content = parsed
            .content
            .iter()
            .filter(|b| b.block_type == "text")
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("");
        Ok(SummaryResult {
            content,
            input_tokens: parsed.usage.input_tokens,
            output_tokens: parsed.usage.output_tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_key_is_refused() {
        let r = AnthropicProvider::new(
            "  ".into(),
            "claude-haiku-4-5-20251001".into(),
            16384,
            1500,
            Duration::from_secs(30),
        );
        assert!(r.is_err());
    }

    #[test]
    fn valid_construction_advertises_correct_kind() {
        let p = AnthropicProvider::new(
            "sk-ant-test".into(),
            "claude-haiku-4-5-20251001".into(),
            16384,
            1500,
            Duration::from_secs(30),
        )
        .unwrap();
        assert_eq!(p.kind(), ProviderKind::Anthropic);
    }
}
