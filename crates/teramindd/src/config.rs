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
