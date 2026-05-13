//! JSON-RPC contract and cross-platform IPC transport for Teramind.

pub mod client;
pub mod codec;
pub mod error;
pub mod proto;
pub mod server;
pub mod transport;

pub use client::IpcClient;
pub use error::IpcError;
pub use proto::{Notify, Request, Response};
pub use server::IpcServer;
