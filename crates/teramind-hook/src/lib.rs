//! Tiny hook shim binary for routing Claude Code hook events into the Teramind daemon.

pub mod auto_recall;
pub mod hook_input;
pub mod inbox;
pub mod selftest;
pub mod spawn;
pub mod team_share_prompt;
pub mod translate;

/// Shared test lock for tests that mutate process-wide env vars (HOME, XDG_DATA_HOME).
/// Both `inbox` and `translate` tests acquire this before touching env vars.
#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
