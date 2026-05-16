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
