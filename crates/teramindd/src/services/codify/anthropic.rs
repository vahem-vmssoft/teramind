//! Anthropic codify provider (gated by network_egress=true + anthropic_api_key).

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::path::Path;
use std::time::Duration;
use teramind_core::codify::{CodifyProvider, CodifyRequest, CodifyResult};

pub struct AnthropicCodifyProvider {
    api_key: String,
    model: String,
    http: reqwest::Client,
}

impl AnthropicCodifyProvider {
    pub fn try_new(secrets_path: &Path, model: String) -> anyhow::Result<Self> {
        #[derive(Deserialize)]
        struct Secrets {
            #[serde(default)] network_egress: bool,
            anthropic_api_key: Option<String>,
        }
        if !secrets_path.exists() {
            anyhow::bail!("Anthropic codify provider requires {} with network_egress=true + anthropic_api_key", secrets_path.display());
        }
        let raw = std::fs::read_to_string(secrets_path)?;
        let s: Secrets = toml::from_str(&raw)?;
        if !s.network_egress {
            anyhow::bail!("network_egress must be true to enable Anthropic codify provider");
        }
        let key = s.anthropic_api_key.ok_or_else(|| anyhow::anyhow!("missing anthropic_api_key"))?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()?;
        Ok(Self { api_key: key, model, http })
    }
}

#[derive(Deserialize)]
struct Msg { content: Vec<MsgPart>, usage: Usage }
#[derive(Deserialize)]
struct MsgPart { text: Option<String> }
#[derive(Deserialize)]
struct Usage { input_tokens: u32, output_tokens: u32 }

#[async_trait]
impl CodifyProvider for AnthropicCodifyProvider {
    async fn codify(&self, req: CodifyRequest) -> anyhow::Result<CodifyResult> {
        use crate::services::codify::ollama::parse_decision;
        use crate::services::codify::prompts::SYSTEM_PROMPT;

        let user_prompt = format!(
            "Observation kind: {}\nFrequency: {}\nProject cwds: {:?}\n\nBundled context:\n---\n{}\n---\n\nReturn JSON now.",
            req.observation_kind, req.frequency, req.cwds, req.bundled_context,
        );
        let body = json!({
            "model": self.model,
            "system": SYSTEM_PROMPT,
            "messages": [{"role":"user","content": user_prompt}],
            "max_tokens": req.max_output_tokens as i32,
        });
        let resp = self.http.post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body).send().await?
            .error_for_status()?;
        let m: Msg = resp.json().await?;
        let text = m.content.into_iter().find_map(|p| p.text).unwrap_or_default();
        let decision = parse_decision(&text)?;
        Ok(CodifyResult {
            decision,
            input_tokens: m.usage.input_tokens,
            output_tokens: m.usage.output_tokens,
        })
    }
    fn name(&self) -> &str { "anthropic" }
}
