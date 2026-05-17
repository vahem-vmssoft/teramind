//! Teramind MCP stdio server.
//!
//! Exposes `search`, `recall`, and `save_skill` MCP tools that forward
//! requests to the local Teramind daemon over IPC.

pub mod server;
pub mod transport_https;
pub mod transport_local;
