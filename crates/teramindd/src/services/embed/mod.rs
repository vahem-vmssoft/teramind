//! Embedding provider implementations. Each provider lives in its own
//! module; the factory + config loader arrive in a later section.

pub mod ollama;
pub mod fastembed_local;
pub mod cloud;
pub mod factory;

pub use factory::build_provider;

use teramind_core::embed::ProviderKind;

#[derive(Debug, Clone, Copy)]
pub struct ModelMeta {
    pub dimension: usize,
    pub max_tokens: usize,
}

pub fn model_meta(provider: ProviderKind, model: &str) -> ModelMeta {
    match (provider, model) {
        (ProviderKind::Ollama, "nomic-embed-text-v2-moe") => ModelMeta { dimension: 768, max_tokens: 8192 },
        (ProviderKind::Ollama, "nomic-embed-text")        => ModelMeta { dimension: 768, max_tokens: 8192 },
        (ProviderKind::Ollama, "mxbai-embed-large")       => ModelMeta { dimension: 1024, max_tokens: 512 },
        (ProviderKind::Fastembed, _)                       => ModelMeta { dimension: 768, max_tokens: 8192 },
        _                                                  => ModelMeta { dimension: 768, max_tokens: 8192 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_ollama_model_returns_correct_dim() {
        let m = model_meta(ProviderKind::Ollama, "nomic-embed-text-v2-moe");
        assert_eq!(m.dimension, 768);
        assert_eq!(m.max_tokens, 8192);
    }

    #[test]
    fn unknown_model_falls_back_to_768() {
        let m = model_meta(ProviderKind::Ollama, "no-such-model");
        assert_eq!(m.dimension, 768);
    }

    #[test]
    fn mxbai_has_1024_dim() {
        let m = model_meta(ProviderKind::Ollama, "mxbai-embed-large");
        assert_eq!(m.dimension, 1024);
    }
}
