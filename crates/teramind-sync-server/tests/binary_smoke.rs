//! Confirms the compiled binary's CLI surface is wired up.

use std::process::Command;

fn binary() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_teramind-sync-server"))
}

#[test]
fn version_subcommand_prints_version() {
    let out = Command::new(binary()).arg("version").output().unwrap();
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.starts_with("teramind-sync-server "));
}

#[test]
fn help_lists_subcommands() {
    let out = Command::new(binary()).arg("--help").output().unwrap();
    let s = String::from_utf8(out.stdout).unwrap();
    for sub in &["serve", "migrate", "invite", "member", "version"] {
        assert!(s.contains(sub), "--help must list `{sub}`");
    }
}
