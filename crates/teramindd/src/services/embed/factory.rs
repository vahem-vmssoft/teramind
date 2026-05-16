//! Provider factory. Reads EmbedConfig, constructs the matching impl.

use crate::config::EmbedConfig;
use crate::services::embed::{model_meta, cloud::CloudProvider, fastembed_local::FastEmbedProvider, ollama::OllamaProvider};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::embed::{EmbeddingProvider, ProviderKind};

pub fn build_provider(cfg: &EmbedConfig) -> anyhow::Result<Arc<dyn EmbeddingProvider>> {
    cfg.validate()?;
    let meta = model_meta(cfg.provider, &cfg.model);
    match cfg.provider {
        ProviderKind::Ollama => {
            let timeout = Duration::from_millis(cfg.ollama.request_timeout_ms);
            Ok(Arc::new(OllamaProvider::new(
                cfg.ollama.url.clone(),
                cfg.model.clone(),
                meta.dimension,
                meta.max_tokens,
                timeout,
            )))
        }
        ProviderKind::Fastembed => {
            let cache_dir = cfg.fastembed.cache_dir.clone()
                .map(PathBuf::from)
                .unwrap_or_else(default_fastembed_cache_dir);
            std::fs::create_dir_all(&cache_dir).ok();
            let p = FastEmbedProvider::new_default(cache_dir)
                .map_err(|e| anyhow::anyhow!("fastembed init: {e}"))?;
            Ok(Arc::new(p))
        }
        kind @ (ProviderKind::Anthropic | ProviderKind::Openai | ProviderKind::Voyage) => {
            let p = CloudProvider::new(kind, cfg.model.clone())
                .map_err(|e| anyhow::anyhow!("cloud provider init: {e}"))?;
            Ok(Arc::new(p))
        }
    }
}

fn default_fastembed_cache_dir() -> PathBuf {
    #[cfg(unix)] {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        home.join(".local/share/teramind/embed-models")
    }
    #[cfg(windows)] {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default();
        local.join("teramind").join("embed-models")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_ollama_provider_with_defaults() {
        let cfg = EmbedConfig::default();
        let p = build_provider(&cfg).expect("ollama default");
        assert_eq!(p.kind(), ProviderKind::Ollama);
        assert_eq!(p.dimension(), 768);
    }

    #[test]
    fn build_cloud_without_egress_fails() {
        let mut cfg = EmbedConfig::default();
        cfg.provider = ProviderKind::Anthropic;
        cfg.network_egress = false;
        assert!(build_provider(&cfg).is_err());
    }

    #[test]
    fn build_cloud_with_egress_succeeds_but_health_fails() {
        let mut cfg = EmbedConfig::default();
        cfg.provider = ProviderKind::Anthropic;
        cfg.network_egress = true;
        let p = build_provider(&cfg).expect("config validates");
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let r = rt.block_on(p.health_check());
        assert!(r.is_err());
    }
}
