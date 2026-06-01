#![cfg(unix)]
//! summarizer §10 — `teramind doctor` surfaces summary provider health, backlog
//! count, and summaries-written/errors counters.
//!
//! Daemon up against external PG (when TERAMIND_TEST_PG_URL is provided);
//! otherwise embedded PG. We `init` + `start`, poll `status` until uptime is
//! reported, then exec `doctor` and pin its summarizer surfaces.

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

#[test]
fn doctor_surfaces_summary_provider_and_backlog() {
    let _ = Command::new("cargo")
        .args(["build", "-p", "teramindd", "--bin", "teramindd"])
        .status();

    let tmp = tempdir().unwrap();
    let target_dir = cargo_bin("teramind").parent().unwrap().to_path_buf();
    let path = format!(
        "{}:{}",
        target_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let mut env: Vec<(&'static str, String)> = vec![
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
        ("PATH", path),
    ];
    if let Ok(url) = std::env::var("TERAMIND_TEST_PG_URL") {
        env.push(("TERAMIND_PG_URL", url));
    }

    let teramind = cargo_bin("teramind");

    assert!(Command::new(&teramind)
        .arg("init")
        .envs(env.iter().cloned())
        .status()
        .unwrap()
        .success());
    let _ = Command::new(&teramind)
        .arg("start")
        .envs(env.iter().cloned())
        .status();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let out = Command::new(&teramind)
            .arg("status")
            .envs(env.iter().cloned())
            .output()
            .unwrap();
        if out.status.success() && String::from_utf8_lossy(&out.stdout).contains("uptime") {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!("daemon never became responsive");
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    let out = Command::new(&teramind)
        .arg("doctor")
        .envs(env.iter().cloned())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();

    let has_summary_line = stdout
        .lines()
        .any(|l| l.contains("summary") || l.contains("summarizer"));
    let has_backlog_line = stdout.lines().any(|l| l.contains("backlog"));

    // Cleanup
    if let Ok(pid_str) = std::fs::read_to_string(tmp.path().join("xdg-data/teramind/teramindd.pid"))
    {
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }

    assert!(
        has_summary_line,
        "doctor stdout missing summary/summarizer line:\n{stdout}"
    );
    assert!(
        has_backlog_line,
        "doctor stdout missing backlog line:\n{stdout}"
    );
}
