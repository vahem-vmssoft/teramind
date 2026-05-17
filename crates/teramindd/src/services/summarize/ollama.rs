//! Ollama chat-completion provider (POST /api/chat).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use teramind_core::summarize::{ProviderKind, SummaryError, SummaryProvider, SummaryResult};

#[derive(Clone)]
pub struct OllamaChatProvider {
    url: String,
    model: String,
    max_input_tokens: usize,
    max_output_tokens: usize,
    client: reqwest::Client,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: [Message<'a>; 2],
    stream: bool,
    options: ChatOptions,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatOptions {
    num_predict: i32,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ResponseMessage,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

#[derive(Deserialize)]
struct VersionResponse {
    #[serde(default)]
    #[allow(dead_code)]
    version: String,
}

impl OllamaChatProvider {
    pub fn new(
        url: String,
        model: String,
        max_input_tokens: usize,
        max_output_tokens: usize,
        timeout: Duration,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("reqwest client build");
        Self {
            url,
            model,
            max_input_tokens,
            max_output_tokens,
            client,
        }
    }
}

#[async_trait]
impl SummaryProvider for OllamaChatProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Ollama
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
        let url = format!("{}/api/version", self.url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| SummaryError::Unhealthy(format!("GET {url}: {e}")))?;
        if !resp.status().is_success() {
            return Err(SummaryError::Unhealthy(format!(
                "ollama version returned {}",
                resp.status()
            )));
        }
        let _: VersionResponse = resp
            .json()
            .await
            .map_err(|e| SummaryError::Unhealthy(format!("decode version: {e}")))?;
        Ok(())
    }

    async fn summarize(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        max_output_tokens: usize,
    ) -> Result<SummaryResult, SummaryError> {
        let url = format!("{}/api/chat", self.url);
        let req = ChatRequest {
            model: &self.model,
            messages: [
                Message {
                    role: "system",
                    content: system_prompt,
                },
                Message {
                    role: "user",
                    content: user_prompt,
                },
            ],
            stream: false,
            options: ChatOptions {
                num_predict: max_output_tokens as i32,
            },
        };
        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| SummaryError::Network(format!("POST {url}: {e}")))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(SummaryError::ModelNotFound(self.model.clone()));
        }
        if !resp.status().is_success() {
            return Err(SummaryError::Other(format!(
                "ollama chat returned {}",
                resp.status()
            )));
        }
        let body: ChatResponse = resp
            .json()
            .await
            .map_err(|e| SummaryError::Other(format!("decode chat: {e}")))?;
        Ok(SummaryResult {
            content: body.message.content,
            input_tokens: body.prompt_eval_count.unwrap_or(0),
            output_tokens: body.eval_count.unwrap_or(0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_advertises_correct_kind() {
        let p = OllamaChatProvider::new(
            "http://localhost:11434".into(),
            "qwen3.6:latest".into(),
            16384,
            1500,
            Duration::from_secs(60),
        );
        assert_eq!(p.kind(), ProviderKind::Ollama);
        assert_eq!(p.model_id(), "qwen3.6:latest");
        assert_eq!(p.max_input_tokens(), 16384);
        assert_eq!(p.max_output_tokens(), 1500);
    }
}
