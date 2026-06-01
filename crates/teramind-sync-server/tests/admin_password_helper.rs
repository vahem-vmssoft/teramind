//! Dashboard §4 — `teramind-sync-server admin-password` is a one-shot CLI
//! helper: it hashes a password (argon2id) and emits a TOML snippet ready to
//! paste into the [admin] block. The snippet must include BOTH an
//! `admin_password_hash` (starting with `$argon2id$`) AND a fresh
//! `admin_session_secret` line.
//!
//! Interactive prompting can't run in a test harness, so the subcommand
//! accepts `--password <value>` for non-interactive use (humans still get
//! the prompted flow when --password is omitted).

use std::process::Command;

fn binary() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_teramind-sync-server"))
}

#[test]
fn admin_password_prints_argon2id_hash_and_session_secret() {
    let out = Command::new(binary())
        .arg("admin-password")
        .arg("--password")
        .arg("verylongtestpassword")
        .output()
        .expect("running admin-password must not fail to spawn");
    assert!(
        out.status.success(),
        "admin-password must exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);

    // argon2id hash — PHC string starts with `$argon2id$`.
    assert!(
        stdout.contains("$argon2id$"),
        "stdout must contain an argon2id hash starting with `$argon2id$`; got:\n{stdout}"
    );
    // Session secret line — production code writes `admin_session_secret = "<hex>"`.
    assert!(
        stdout.contains("admin_session_secret"),
        "stdout must contain an `admin_session_secret` line; got:\n{stdout}"
    );
    // The hash key must also be emitted as a TOML field.
    assert!(
        stdout.contains("admin_password_hash"),
        "stdout must contain an `admin_password_hash` line; got:\n{stdout}"
    );
}

#[test]
fn admin_password_rejects_short_password() {
    // < 12 chars: the helper must bail out non-zero.
    let out = Command::new(binary())
        .arg("admin-password")
        .arg("--password")
        .arg("short")
        .output()
        .expect("spawn");
    assert!(
        !out.status.success(),
        "admin-password must reject passwords shorter than 12 characters"
    );
}
