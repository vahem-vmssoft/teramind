//! Pure-Rust self-update logic. The CLI wrapper lives in
//! `commands/self_update.rs`; everything testable without an HTTP server
//! lives here so we can drive it from a tempdir-based test harness.

pub mod release_index;
