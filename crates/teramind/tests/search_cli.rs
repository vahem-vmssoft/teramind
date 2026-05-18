#![cfg(unix)]
use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::tempdir;

fn cargo_bin(name: &str) -> std::path::PathBuf {
    std::env::var(format!("CARGO_BIN_EXE_{name}"))
        .map(Into::into)
        .unwrap_or_else(|_| {
            // CARGO_TARGET_DIR may be set explicitly; otherwise derive from CARGO_MANIFEST_DIR
            // (which is the crate dir) by walking up to the workspace root where `target/` lives.
            let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| {
                let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
                let workspace_root = std::path::PathBuf::from(&manifest)
                    .ancestors()
                    .find(|p| p.join("Cargo.toml").exists() && p.join("Cargo.lock").exists())
                    .unwrap_or_else(|| std::path::Path::new(&manifest))
                    .to_path_buf();
                workspace_root.join("target").to_string_lossy().into_owned()
            });
            let profile = if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            };
            std::path::PathBuf::from(target).join(profile).join(name)
        })
}

#[test]
fn teramind_search_returns_seeded_hit() {
    let _ = Command::new("cargo")
        .args(["build", "--workspace"])
        .status();

    let tmp = tempdir().unwrap();
    let target_dir = cargo_bin("teramind").parent().unwrap().to_path_buf();
    let path_with_target = format!(
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
        ("PATH", path_with_target),
    ];
    // If the test fixture is pointing at an external Postgres, let the
    // daemon use it too — embedded PG bootstrap blows past the start
    // deadline on first boot and makes this test flake.
    if let Ok(url) = std::env::var("TERAMIND_TEST_PG_URL") {
        env.push(("TERAMIND_PG_URL", url));
    }

    let teramind = cargo_bin("teramind");
    let hook = cargo_bin("teramind-hook");

    assert!(Command::new(&teramind)
        .arg("init")
        .envs(env.iter().cloned())
        .status()
        .unwrap()
        .success());
    Command::new(&teramind)
        .arg("start")
        .envs(env.iter().cloned())
        .status()
        .unwrap();
    let mut ready = false;
    for _ in 0..90 {
        let out = Command::new(&teramind)
            .arg("status")
            .envs(env.iter().cloned())
            .output()
            .unwrap();
        if String::from_utf8_lossy(&out.stdout).contains("uptime") {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    assert!(ready, "daemon never became responsive");

    for payload in &[
        r#"{"hook_event_name":"SessionStart","session_id":"cli-test","cwd":"/tmp/cli-test","source":"startup"}"#,
        r#"{"hook_event_name":"UserPromptSubmit","session_id":"cli-test","cwd":"/tmp/cli-test","prompt":"rust async deadlock"}"#,
    ] {
        let mut child = Command::new(&hook)
            .envs(env.iter().cloned())
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
        assert!(child.wait().unwrap().success());
    }
    std::thread::sleep(std::time::Duration::from_secs(2));

    // The daemon refreshes the MV every 30s; poll for up to 40s.
    let mut found = false;
    for _ in 0..40 {
        let out = Command::new(&teramind)
            .args(["search", "deadlock"])
            .envs(env.iter().cloned())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        if stdout.contains("deadlock") {
            found = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }

    // Cleanup daemon.
    if let Ok(pid_str) = std::fs::read_to_string(tmp.path().join("xdg-data/teramind/teramindd.pid"))
    {
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }
    assert!(
        found,
        "teramind search 'deadlock' did not find the seeded prompt"
    );
}
