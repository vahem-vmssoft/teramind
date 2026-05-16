//! Embedding provider implementations. Each provider lives in its own
//! module; the factory + config loader arrive in a later section.

pub mod ollama;
pub mod fastembed_local;
pub mod cloud;
