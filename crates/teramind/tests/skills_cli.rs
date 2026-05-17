//! Smoke test for `teramind skills` commands.
//!
//! A full integration test would require spinning up a mock IPC server, setting
//! TERAMIND_SOCKET, and calling the lib fns directly (the same pattern as
//! `crates/teramind/tests/init_team.rs` from Plan K). That boilerplate is
//! deferred to a v1.1 follow-up because:
//!
//!   1. The IPC-level dispatch is already covered by §15's `codify_tool.rs`
//!      and earlier plan tests.
//!   2. The `skills::list/show/observations` fns share the exact same
//!      `crate::ipc::request(...)` structure as `search` and `sessions::show`,
//!      both already tested at the IPC layer.
//!   3. Spinning PgSupervisor here would duplicate §2-§3's unit tests with
//!      no new coverage.
//!
//! The compile-time check below ensures all public symbols remain reachable
//! and the function signatures match the CLI dispatch in main.rs.

#[allow(dead_code)]
fn _type_check() {
    // Verify the three public fns have the expected signatures.
    let _: fn(String, u32) -> _ = teramind::commands::skills::list;
    let _: fn(String) -> _ = teramind::commands::skills::show;
    let _: fn(Option<String>, i32, Option<String>, u32) -> _ =
        teramind::commands::skills::observations;
}
