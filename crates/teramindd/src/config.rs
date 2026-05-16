use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "Config::default_ingest_queue_capacity")]
    pub ingest_queue_capacity: usize,
    #[serde(default = "Config::default_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
    #[serde(default = "Config::default_redaction_enabled")]
    pub redaction_enabled: bool,
    #[serde(default = "Config::default_autorecall_enabled")]
    pub autorecall_enabled: bool,
    #[serde(default = "Config::default_storage_sample_interval_secs")]
    pub storage_sample_interval_secs: u64,
    #[serde(default = "Config::default_fs_debounce_ms")]
    pub fs_debounce_ms: u64,
    #[serde(default = "Config::default_attribution_window_ms")]
    pub fs_attribution_window_ms: u64,
    #[serde(default = "Config::default_snapshot_ttl_secs")]
    pub fs_snapshot_ttl_secs: u64,
}

impl Config {
    fn default_ingest_queue_capacity() -> usize {
        4096
    }
    fn default_idle_timeout_secs() -> u64 {
        30 * 60
    }
    fn default_redaction_enabled() -> bool {
        true
    }
    fn default_autorecall_enabled() -> bool {
        true
    }
    fn default_storage_sample_interval_secs() -> u64 {
        300
    }
    fn default_fs_debounce_ms() -> u64 {
        200
    }
    fn default_attribution_window_ms() -> u64 {
        5_000
    }
    fn default_snapshot_ttl_secs() -> u64 {
        1_800
    }

    pub fn defaults() -> Self {
        toml::from_str("").expect("default config must parse from empty toml")
    }

    pub fn load_or_default(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::defaults());
        }
        let text = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }
}

// ── EmbedConfig ──────────────────────────────────────────────────────────────

use teramind_core::embed::ProviderKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedConfig {
    #[serde(default = "default_provider")]
    pub provider: ProviderKind,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_batch_size")]
    pub batch_size: u32,
    #[serde(default = "default_max_throughput")]
    pub max_throughput_per_min: u32,
    #[serde(default = "default_orphan_sweep")]
    pub orphan_sweep_interval_hr: u32,
    #[serde(default)]
    pub network_egress: bool,
    #[serde(default)]
    pub ollama: OllamaConfig,
    #[serde(default)]
    pub fastembed: FastembedConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaConfig {
    #[serde(default = "default_ollama_url")]
    pub url: String,
    #[serde(default = "default_ollama_timeout_ms")]
    pub request_timeout_ms: u64,
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            url: default_ollama_url(),
            request_timeout_ms: default_ollama_timeout_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FastembedConfig {
    #[serde(default)]
    pub cache_dir: Option<String>,
}

fn default_provider() -> ProviderKind { ProviderKind::Ollama }
fn default_model() -> String { "nomic-embed-text-v2-moe".into() }
fn default_poll_interval() -> u64 { 5 }
fn default_batch_size() -> u32 { 32 }
fn default_max_throughput() -> u32 { 1000 }
fn default_orphan_sweep() -> u32 { 24 }
fn default_ollama_url() -> String { "http://localhost:11434".into() }
fn default_ollama_timeout_ms() -> u64 { 10_000 }

impl Default for EmbedConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            model: default_model(),
            poll_interval_secs: default_poll_interval(),
            batch_size: default_batch_size(),
            max_throughput_per_min: default_max_throughput(),
            orphan_sweep_interval_hr: default_orphan_sweep(),
            network_egress: false,
            ollama: OllamaConfig::default(),
            fastembed: FastembedConfig::default(),
        }
    }
}

impl EmbedConfig {
    pub fn load_or_default(path: &std::path::Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let body = std::fs::read_to_string(path)?;
        let c: Self = toml::from_str(&body)?;
        c.validate()?;
        Ok(c)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.provider.is_cloud() && !self.network_egress {
            anyhow::bail!(
                "embed.toml: provider={:?} requires network_egress=true. \
                 Flip the flag or switch to ollama/fastembed.",
                self.provider,
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod embed_config_tests {
    use super::*;

    #[test]
    fn default_is_ollama_with_v2_moe() {
        let c = EmbedConfig::default();
        assert!(matches!(c.provider, ProviderKind::Ollama));
        assert_eq!(c.model, "nomic-embed-text-v2-moe");
        assert!(!c.network_egress);
    }

    #[test]
    fn cloud_provider_requires_network_egress() {
        let mut c = EmbedConfig::default();
        c.provider = ProviderKind::Anthropic;
        assert!(c.validate().is_err());
        c.network_egress = true;
        c.validate().expect("should pass with egress=true");
    }

    #[test]
    fn local_providers_dont_require_egress() {
        for p in [ProviderKind::Ollama, ProviderKind::Fastembed] {
            let mut c = EmbedConfig::default();
            c.provider = p;
            c.network_egress = false;
            c.validate().expect("local provider should pass");
        }
    }

    #[test]
    fn load_returns_default_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let c = EmbedConfig::load_or_default(&dir.path().join("embed.toml")).unwrap();
        assert_eq!(c.model, "nomic-embed-text-v2-moe");
    }
}

// ── SearchWeights config ──────────────────────────────────────────────────────

use crate::services::search::BlendWeights;

#[derive(Debug, Clone, Deserialize)]
struct SearchFile {
    #[serde(default)]
    blend: BlendOverride,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct BlendOverride {
    fts: Option<f32>,
    trgm: Option<f32>,
    semantic: Option<f32>,
    recency: Option<f32>,
    project: Option<f32>,
}

pub fn load_search_weights(path: &std::path::Path) -> anyhow::Result<BlendWeights> {
    if !path.exists() {
        return Ok(BlendWeights::default());
    }
    let body = std::fs::read_to_string(path)?;
    let f: SearchFile = toml::from_str(&body)?;
    let d = BlendWeights::default();
    Ok(BlendWeights {
        fts:      f.blend.fts.unwrap_or(d.fts),
        trgm:     f.blend.trgm.unwrap_or(d.trgm),
        semantic: f.blend.semantic.unwrap_or(d.semantic),
        recency:  f.blend.recency.unwrap_or(d.recency),
        project:  f.blend.project.unwrap_or(d.project),
    })
}

#[cfg(test)]
mod search_weights_tests {
    use super::*;

    #[test]
    fn missing_file_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let w = load_search_weights(&dir.path().join("search.toml")).unwrap();
        assert_eq!(w.semantic, 0.0);
    }

    #[test]
    fn partial_override_keeps_other_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("search.toml");
        std::fs::write(&path, "[blend]\nsemantic = 0.4\n").unwrap();
        let w = load_search_weights(&path).unwrap();
        assert!((w.semantic - 0.4).abs() < 1e-6);
        assert!((w.fts - 0.6).abs() < 1e-6);
    }
}

// ── Existing tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_sane() {
        let c = Config::defaults();
        assert!(c.ingest_queue_capacity >= 1024);
        assert!(c.redaction_enabled);
    }

    #[test]
    fn fs_watcher_defaults_match_spec() {
        let c = Config::defaults();
        assert_eq!(c.fs_debounce_ms, 200);
        assert_eq!(c.fs_attribution_window_ms, 5_000);
        assert_eq!(c.fs_snapshot_ttl_secs, 1_800);
    }
}

// ============================ summarize config ============================

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SummarizeConfig {
    #[serde(default = "default_summarize_provider")]
    pub provider: teramind_core::embed::ProviderKind,
    #[serde(default = "default_summarize_model")]
    pub model: String,
    #[serde(default = "default_summarize_poll")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_summarize_min_turns")]
    pub min_turns: u32,
    #[serde(default = "default_summarize_min_duration")]
    pub min_duration_secs: u64,
    #[serde(default = "default_summarize_input_chars")]
    pub input_char_budget: u32,
    #[serde(default = "default_summarize_output_tokens")]
    pub output_token_budget: u32,
    #[serde(default)]
    pub network_egress: bool,
    #[serde(default)]
    pub ollama: SummarizeOllama,
    #[serde(default)]
    pub anthropic: SummarizeAnthropic,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SummarizeOllama {
    #[serde(default = "default_summarize_ollama_url")]
    pub url: String,
    #[serde(default = "default_summarize_ollama_timeout")]
    pub request_timeout_ms: u64,
}

impl Default for SummarizeOllama {
    fn default() -> Self {
        Self {
            url: default_summarize_ollama_url(),
            request_timeout_ms: default_summarize_ollama_timeout(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct SummarizeAnthropic {
    #[serde(default = "default_anthropic_key_field")]
    pub api_key_field: String,
    #[serde(default = "default_anthropic_timeout")]
    pub request_timeout_ms: u64,
}

fn default_summarize_provider() -> teramind_core::embed::ProviderKind {
    teramind_core::embed::ProviderKind::Ollama
}
fn default_summarize_model() -> String { "qwen3.6:latest".into() }
fn default_summarize_poll() -> u64 { 30 }
fn default_summarize_min_turns() -> u32 { 3 }
fn default_summarize_min_duration() -> u64 { 60 }
fn default_summarize_input_chars() -> u32 { 16000 }
fn default_summarize_output_tokens() -> u32 { 1500 }
fn default_summarize_ollama_url() -> String { "http://localhost:11434".into() }
fn default_summarize_ollama_timeout() -> u64 { 60_000 }
fn default_anthropic_key_field() -> String { "anthropic_api_key".into() }
fn default_anthropic_timeout() -> u64 { 30_000 }

impl Default for SummarizeConfig {
    fn default() -> Self {
        Self {
            provider: default_summarize_provider(),
            model: default_summarize_model(),
            poll_interval_secs: default_summarize_poll(),
            min_turns: default_summarize_min_turns(),
            min_duration_secs: default_summarize_min_duration(),
            input_char_budget: default_summarize_input_chars(),
            output_token_budget: default_summarize_output_tokens(),
            network_egress: false,
            ollama: SummarizeOllama::default(),
            anthropic: SummarizeAnthropic::default(),
        }
    }
}

impl SummarizeConfig {
    pub fn load_or_default(path: &std::path::Path) -> anyhow::Result<Self> {
        if !path.exists() { return Ok(Self::default()); }
        let body = std::fs::read_to_string(path)?;
        let c: Self = toml::from_str(&body)?;
        c.validate()?;
        Ok(c)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.provider.is_cloud() && !self.network_egress {
            anyhow::bail!(
                "summarize.toml: provider={:?} requires network_egress=true. \
                 Flip the flag or switch to ollama.",
                self.provider,
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod summarize_config_tests {
    use super::*;
    use teramind_core::embed::ProviderKind;

    #[test]
    fn default_is_ollama_with_qwen36() {
        let c = SummarizeConfig::default();
        assert!(matches!(c.provider, ProviderKind::Ollama));
        assert_eq!(c.model, "qwen3.6:latest");
        assert_eq!(c.min_turns, 3);
        assert_eq!(c.min_duration_secs, 60);
    }

    #[test]
    fn cloud_provider_requires_network_egress() {
        let mut c = SummarizeConfig::default();
        c.provider = ProviderKind::Anthropic;
        assert!(c.validate().is_err());
        c.network_egress = true;
        c.validate().expect("ok with egress=true");
    }

    #[test]
    fn local_providers_dont_require_egress() {
        let mut c = SummarizeConfig::default();
        c.provider = ProviderKind::Ollama;
        c.network_egress = false;
        c.validate().expect("ollama ok");
    }
}
