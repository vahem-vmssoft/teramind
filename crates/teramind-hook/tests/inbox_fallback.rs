#![cfg(unix)]
use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::tempdir;

fn cargo_bin(name: &str) -> std::path::PathBuf {
    std::env::var(format!("CARGO_BIN_EXE_{name}"))
        .map(Into::into)
        .unwrap_or_else(|_| {
            let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
            let profile = if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            };
            std::path::PathBuf::from(target).join(profile).join(name)
        })
}

#[test]
fn hook_writes_to_inbox_when_daemon_unreachable() {
    let _ = Command::new("cargo")
        .args(["build", "-p", "teramind-hook"])
        .status();

    let tmp = tempdir().unwrap();
    let sock = tmp.path().join("no-such.sock");
    let xdg = tmp.path().join("xdg-data");

    let payload = r#"{"hook_event_name":"UserPromptSubmit","session_id":"inbox-test","cwd":"/w","prompt":"hi"}"#;
    let hook = cargo_bin("teramind-hook");
    let mut child = Command::new(&hook)
        .env("TERAMIND_SOCKET", sock.to_string_lossy().to_string())
        .env("HOME", tmp.path())
        .env("XDG_DATA_HOME", &xdg)
        .env("TERAMIND_HOOK_NO_SPAWN", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    assert!(
        child.wait().unwrap().success(),
        "hook must exit 0 even when daemon is down"
    );

    let inbox_dir = xdg.join("teramind").join("inbox");
    assert!(inbox_dir.exists(), "inbox dir not created");
    let files: Vec<_> = std::fs::read_dir(&inbox_dir)
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert_eq!(
        files.len(),
        1,
        "expected exactly one inbox file, found {}",
        files.len()
    );
}
