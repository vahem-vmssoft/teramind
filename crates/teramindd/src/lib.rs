//! Teramind daemon: long-running process owning Postgres, ingest, FS watcher, IPC server.

pub mod app;
pub mod config;
pub mod paths;
pub mod services;
pub mod signals;

pub use crate::services::ingest::{route_with_deps, IngestAuth, RouteDeps};
pub use crate::services::rpc_dispatch::{dispatch, AuthContext, RpcDeps};
