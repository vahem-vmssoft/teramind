//! Session summarizer: trait + provider impls + digest + prompts.

pub mod anthropic;
pub mod digest;
pub mod factory;
pub mod ollama;
pub mod openai;
pub mod prompts;

pub use factory::build_provider;
pub mod null;
