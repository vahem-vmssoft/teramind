//! Ollama-backed codify provider. Reuses the same HTTP shape Plan H's
//! OllamaChatProvider uses (POST /api/chat with non-streaming JSON output).

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;
use teramind_core::codify::{CodifyDecision, CodifyProvider, CodifyRequest, CodifyResult};

pub struct OllamaCodifyProvider {
    base_url: String,
    model: String,
    http: reqwest::Client,
}

impl OllamaCodifyProvider {
    pub fn new(model: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("reqwest client");
        Self {
            base_url: "http://localhost:11434".into(),
            model,
            http,
        }
    }
}

#[derive(Deserialize)]
struct OllamaChatResponse {
    message: ChatMessage,
    #[serde(default)]
    prompt_eval_count: u32,
    #[serde(default)]
    eval_count: u32,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}

#[async_trait]
impl CodifyProvider for OllamaCodifyProvider {
    async fn codify(&self, req: CodifyRequest) -> anyhow::Result<CodifyResult> {
        use crate::services::codify::prompts::SYSTEM_PROMPT;
        let user_prompt = format!(
            "Observation kind: {}\nFrequency: {}\nProject cwds: {:?}\n\nBundled context:\n---\n{}\n---\n\nReturn JSON now.",
            req.observation_kind, req.frequency, req.cwds, req.bundled_context,
        );
        let body = json!({
            "model": self.model,
            "stream": false,
            "format": "json",
            "messages": [
                { "role": "system", "content": SYSTEM_PROMPT },
                { "role": "user",   "content": user_prompt }
            ],
            "options": { "num_predict": req.max_output_tokens as i32 }
        });

        let resp = self
            .http
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        let parsed: OllamaChatResponse = resp.json().await?;
        let decision = parse_decision(&parsed.message.content)?;
        Ok(CodifyResult {
            decision,
            input_tokens: parsed.prompt_eval_count,
            output_tokens: parsed.eval_count,
        })
    }
    fn name(&self) -> &str {
        "ollama"
    }
}

pub(crate) fn parse_decision(raw: &str) -> anyhow::Result<CodifyDecision> {
    let v: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| anyhow::anyhow!("non-JSON output: {e}"))?;
    let kind = v
        .get("decision")
        .and_then(|d| d.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing decision field"))?;
    match kind {
        "skip" => Ok(CodifyDecision::Skip {
            reason: v
                .get("reason")
                .and_then(|r| r.as_str())
                .unwrap_or("")
                .to_string(),
        }),
        "skill" => Ok(CodifyDecision::Skill {
            name: v["name"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing name"))?
                .to_string(),
            description: v["description"].as_str().unwrap_or("").to_string(),
            body: v["body"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing body"))?
                .to_string(),
            applies_to_cwds: v["applies_to_cwds"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
        }),
        other => Err(anyhow::anyhow!("unknown decision: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use teramind_core::codify::CodifyDecision;

    #[test]
    fn parse_skip_round_trips() {
        let raw = r#"{"decision":"skip","reason":"trivial"}"#;
        match parse_decision(raw).unwrap() {
            CodifyDecision::Skip { reason } => assert_eq!(reason, "trivial"),
            _ => panic!("expected Skip"),
        }
    }

    #[test]
    fn parse_skill_round_trips() {
        let raw = r##"{"decision":"skill","name":"rust-pr-prep","description":"d","body":"# x","applies_to_cwds":["/p"]}"##;
        match parse_decision(raw).unwrap() {
            CodifyDecision::Skill {
                name,
                body,
                applies_to_cwds,
                ..
            } => {
                assert_eq!(name, "rust-pr-prep");
                assert_eq!(body, "# x");
                assert_eq!(applies_to_cwds, vec!["/p".to_string()]);
            }
            _ => panic!("expected Skill"),
        }
    }

    #[test]
    fn parse_unknown_decision_errors() {
        let raw = r#"{"decision":"unknown"}"#;
        assert!(parse_decision(raw).is_err());
    }
}
