//! Provider factory. Reads SummarizeConfig, constructs the active provider.

use crate::config::SummarizeConfig;
use crate::services::summarize::{
    anthropic::AnthropicProvider, ollama::OllamaChatProvider, openai::OpenaiProvider,
};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::embed::ProviderKind;
use teramind_core::summarize::SummaryProvider;

pub fn build_provider(
    cfg: &SummarizeConfig,
    secrets_path: &Path,
) -> anyhow::Result<Arc<dyn SummaryProvider>> {
    cfg.validate()?;
    match cfg.provider {
        ProviderKind::Ollama => {
            let timeout = Duration::from_millis(cfg.ollama.request_timeout_ms);
            Ok(Arc::new(OllamaChatProvider::new(
                cfg.ollama.url.clone(),
                cfg.model.clone(),
                cfg.input_char_budget as usize,
                cfg.output_token_budget as usize,
                timeout,
            )))
        }
        ProviderKind::Anthropic => {
            let api_key = read_secret(secrets_path, &cfg.anthropic.api_key_field)?;
            let p = AnthropicProvider::new(
                api_key,
                cfg.model.clone(),
                cfg.input_char_budget as usize,
                cfg.output_token_budget as usize,
                Duration::from_millis(cfg.anthropic.request_timeout_ms),
            ).map_err(|e| anyhow::anyhow!("anthropic init: {e}"))?;
            Ok(Arc::new(p))
        }
        ProviderKind::Openai => {
            Ok(Arc::new(OpenaiProvider::new(cfg.model.clone())))
        }
        ProviderKind::Fastembed | ProviderKind::Voyage => {
            anyhow::bail!(
                "provider {:?} is not valid for summarization. \
                 Use ollama/anthropic/openai.",
                cfg.provider,
            )
        }
    }
}

fn read_secret(path: &Path, field: &str) -> anyhow::Result<String> {
    if !path.exists() {
        anyhow::bail!(
            "secrets file missing: {} (required for cloud providers)",
            path.display(),
        );
    }
    // Enforce 0600 permissions on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)?.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            anyhow::bail!(
                "secrets file {} has insecure permissions ({:o}); chmod 0600 and retry",
                path.display(), mode,
            );
        }
    }
    let body = std::fs::read_to_string(path)?;
    let value: toml::Value = toml::from_str(&body)
        .map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?;
    let s = value.get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("secrets.toml missing field '{field}'"))?
        .to_string();
    if s.trim().is_empty() {
        anyhow::bail!("secrets.toml field '{field}' is empty");
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use teramind_core::embed::ProviderKind;

    #[test]
    fn build_ollama_with_defaults() {
        let cfg = SummarizeConfig::default();
        let secrets = std::path::PathBuf::from("/nonexistent");
        let p = build_provider(&cfg, &secrets).expect("ollama default");
        assert_eq!(p.kind(), ProviderKind::Ollama);
        assert_eq!(p.model_id(), "qwen3.6:latest");
    }

    #[test]
    fn build_anthropic_without_egress_fails() {
        let mut cfg = SummarizeConfig::default();
        cfg.provider = ProviderKind::Anthropic;
        let r = build_provider(&cfg, &std::path::PathBuf::from("/nonexistent"));
        assert!(r.is_err());
    }

    #[test]
    fn build_anthropic_without_secrets_file_fails() {
        let mut cfg = SummarizeConfig::default();
        cfg.provider = ProviderKind::Anthropic;
        cfg.network_egress = true;
        let r = build_provider(&cfg, &std::path::PathBuf::from("/nonexistent"));
        assert!(r.is_err());
        let err = r.err().unwrap();
        let msg = format!("{err}");
        assert!(msg.contains("secrets file missing") || msg.contains("/nonexistent"));
    }

    #[test]
    fn fastembed_is_rejected() {
        let mut cfg = SummarizeConfig::default();
        cfg.provider = ProviderKind::Fastembed;
        let r = build_provider(&cfg, &std::path::PathBuf::from("/x"));
        assert!(r.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn loose_secrets_perms_rejected() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        std::fs::write(&path, "anthropic_api_key = \"sk-ant-test\"").unwrap();
        // Set world-readable (0644).
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let r = read_secret(&path, "anthropic_api_key");
        assert!(r.is_err(), "loose perms should be rejected");
    }

    #[cfg(unix)]
    #[test]
    fn tight_perms_loads_secret() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        std::fs::write(&path, "anthropic_api_key = \"sk-ant-test\"").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let s = read_secret(&path, "anthropic_api_key").unwrap();
        assert_eq!(s, "sk-ant-test");
    }
}
