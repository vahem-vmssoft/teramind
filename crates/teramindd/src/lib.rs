//! Teramind daemon: long-running process owning Postgres, ingest, FS watcher, IPC server.

pub mod app;
pub mod config;
pub mod paths;
pub mod services;
pub mod signals;

pub use crate::services::ingest::{IngestAuth, RouteDeps, route_with_deps};
