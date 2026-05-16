//! Session summarizer: trait + provider impls + digest + prompts.

pub mod digest;
pub mod prompts;
pub mod ollama;
pub mod anthropic;
pub mod openai;
pub mod factory;

pub use factory::build_provider;
