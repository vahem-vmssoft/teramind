//! Teramind daemon: long-running process owning Postgres, ingest, FS watcher, IPC server.

pub mod config;
pub mod paths;
pub mod signals;

// Sections 8+ populate this; placeholder for now so future commits don't need to touch lib.rs.
pub mod services {
    // Filled in by Section 8.
}
