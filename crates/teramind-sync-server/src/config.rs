//! Server configuration loaded from TOML.

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub listen_addr: String,
    pub database_url: String,
    #[serde(default)]
    pub tls: Option<TlsConfig>,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub ingest: IngestConfig,
    #[serde(default)]
    pub admin: Option<AdminConfig>,
    #[serde(default)]
    pub quality: Option<QualityConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TlsConfig {
    pub cert_file: PathBuf,
    pub key_file: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    #[serde(default = "AuthConfig::default_invite_expiry_days")]
    pub invite_default_expires_days: i64,
    #[serde(default = "AuthConfig::default_replay_window")]
    pub proof_replay_window_secs: i64,
    #[serde(default = "AuthConfig::default_replay_size")]
    pub proof_replay_cache_size: usize,
}

impl AuthConfig {
    fn default_invite_expiry_days() -> i64 {
        7
    }
    fn default_replay_window() -> i64 {
        60
    }
    fn default_replay_size() -> usize {
        10_000
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            invite_default_expires_days: Self::default_invite_expiry_days(),
            proof_replay_window_secs: Self::default_replay_window(),
            proof_replay_cache_size: Self::default_replay_size(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct IngestConfig {
    #[serde(default = "IngestConfig::default_batch")]
    pub max_batch_size: usize,
    #[serde(default = "IngestConfig::default_body")]
    pub max_request_body_bytes: usize,
}

impl IngestConfig {
    fn default_batch() -> usize {
        32
    }
    fn default_body() -> usize {
        10 * 1024 * 1024
    }
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            max_batch_size: Self::default_batch(),
            max_request_body_bytes: Self::default_body(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AdminConfig {
    pub admin_password_hash: String,
    pub admin_session_secret: String,
    #[serde(default = "AdminConfig::default_ttl")]
    pub admin_session_ttl_hours: u64,
    #[serde(default = "AdminConfig::default_retention")]
    pub event_log_retention_days: i64,
}

impl AdminConfig {
    fn default_ttl() -> u64 { 12 }
    fn default_retention() -> i64 { 90 }
}

#[derive(Debug, Clone, Deserialize)]
pub struct QualityConfig {
    #[serde(default)]
    pub enabled: bool,
    pub cron: Option<String>,
    #[serde(default)]
    pub baselines: Vec<String>,
    #[serde(default = "QualityConfig::default_binary")]
    pub eval_binary: String,
}

impl QualityConfig {
    fn default_binary() -> String { "teramind-search-eval".into() }
}

impl ServerConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let s = std::fs::read_to_string(path)?;
        let cfg: ServerConfig = toml::from_str(&s)?;
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn loads_minimal() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
listen_addr  = "0.0.0.0:443"
database_url = "postgres://u:p@h/db"
"#
        )
        .unwrap();
        let cfg = ServerConfig::load(f.path()).unwrap();
        assert_eq!(cfg.listen_addr, "0.0.0.0:443");
        assert!(cfg.tls.is_none());
        assert_eq!(cfg.auth.invite_default_expires_days, 7);
        assert_eq!(cfg.ingest.max_batch_size, 32);
    }

    #[test]
    fn loads_with_tls_and_overrides() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
listen_addr  = "0.0.0.0:443"
database_url = "postgres://u:p@h/db"

[tls]
cert_file = "/etc/cert.pem"
key_file  = "/etc/key.pem"

[auth]
proof_replay_window_secs = 30
"#
        )
        .unwrap();
        let cfg = ServerConfig::load(f.path()).unwrap();
        assert!(cfg.tls.is_some());
        assert_eq!(cfg.auth.proof_replay_window_secs, 30);
        assert_eq!(cfg.auth.proof_replay_cache_size, 10_000);
    }
}
