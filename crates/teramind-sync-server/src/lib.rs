//! Teramind central sync server. See docs/superpowers/specs/2026-05-17-teramind-team-sync-design.md.

pub mod admin;
pub mod dashboard_assets;
pub mod admin_api;
pub mod auth;
pub mod config;
pub mod event_log_pruner;
pub mod event_log_writer;
pub mod fts_refresh;
pub mod quality_scheduler;
pub mod handlers;
pub mod invite;
pub mod proof;
pub mod server;
pub mod state;
pub mod tls;
pub mod token;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
