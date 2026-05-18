#![cfg(unix)]
//! End-to-end smoke test exercising `teramind init`, `start`, `status`, `stop`.
//!
//! Runtime is dominated by embedded-PG startup (~10-30s on first run).
//!
//! The test builds `teramindd` itself via `cargo build` so it's self-contained,
//! and prepends `target/<profile>` to `PATH` so the CLI's `which teramindd`
//! fallback locates the daemon binary.

use std::process::Command;
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

fn target_dir() -> std::path::PathBuf {
    let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    std::path::PathBuf::from(target).join(profile)
}

#[test]
fn cli_init_start_status_stop_smoke() {
    // Ensure the daemon binary exists. Best-effort: if this fails the start
    // assertion below will surface a clear error.
    let _ = Command::new("cargo")
        .args(["build", "-p", "teramindd", "--bin", "teramindd"])
        .status();

    let tmp = tempdir().unwrap();
    let target_dir = target_dir();
    let current_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", target_dir.display(), current_path);

    let mut env: Vec<(&str, String)> = vec![
        ("HOME", tmp.path().to_string_lossy().into_owned()),
        (
            "XDG_DATA_HOME",
            tmp.path().join("xdg-data").to_string_lossy().into_owned(),
        ),
        (
            "XDG_CONFIG_HOME",
            tmp.path().join("xdg-config").to_string_lossy().into_owned(),
        ),
        (
            "TERAMIND_SOCKET",
            tmp.path().join("t.sock").to_string_lossy().into_owned(),
        ),
        ("TERAMIND_LOG", "warn".to_string()),
        ("PATH", new_path),
    ];
    // If the test fixture is pointing at an external Postgres, let the
    // daemon use it too — embedded PG boot blows past the start window
    // on first run.
    if let Ok(url) = std::env::var("TERAMIND_TEST_PG_URL") {
        env.push(("TERAMIND_PG_URL", url));
    }

    let teramind = cargo_bin("teramind");

    let out = Command::new(&teramind)
        .arg("init")
        .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // `start` returns success only if the daemon becomes reachable within its
    // own short window (5s); embedded-PG initialization can blow past that on
    // first run. We don't gate on its exit code here — the daemon is spawned
    // detached either way. Instead we poll `status` until it succeeds or we
    // hit a generous timeout.
    let _ = Command::new(&teramind)
        .arg("start")
        .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .output()
        .unwrap();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let status_out = loop {
        let out = Command::new(&teramind)
            .arg("status")
            .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        if out.status.success() && stdout.contains("uptime") {
            break stdout;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "daemon never became responsive within 60s\nlast stdout: {stdout}\nlast stderr: {stderr}"
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    };
    assert!(
        status_out.contains("uptime"),
        "status output did not contain uptime line: {status_out}"
    );

    let _ = Command::new(&teramind)
        .arg("stop")
        .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .output();

    // Belt-and-suspenders: if the daemon didn't shut down cleanly, kill it.
    if let Ok(pid_str) = std::fs::read_to_string(tmp.path().join("xdg-data/teramind/teramindd.pid"))
    {
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }
}
