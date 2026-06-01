//! Shared harness for CLI E2E tests: spin up a daemon, wait for it to be
//! responsive, and connect to its per-instance Postgres database (so the test
//! can seed data the CLI command will then read back).
//!
//! Tests should call `boot_daemon()`, then `connect_daemon_db()`, then exec
//! `cargo_bin("teramind")` with the returned env. `stop_daemon()` on cleanup.

#![cfg(unix)]
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;
use teramind_db::pool::DbPool;

pub fn cargo_bin(name: &str) -> PathBuf {
    std::env::var(format!("CARGO_BIN_EXE_{name}"))
        .map(Into::into)
        .unwrap_or_else(|_| {
            let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| {
                let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
                let workspace_root = PathBuf::from(&manifest)
                    .ancestors()
                    .find(|p| p.join("Cargo.toml").exists() && p.join("Cargo.lock").exists())
                    .unwrap_or_else(|| Path::new(&manifest))
                    .to_path_buf();
                workspace_root.join("target").to_string_lossy().into_owned()
            });
            let profile = if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            };
            PathBuf::from(target).join(profile).join(name)
        })
}

pub struct DaemonHandle {
    pub tmp: TempDir,
    pub env: Vec<(&'static str, String)>,
    pub teramind: PathBuf,
    pub pg_url: Option<String>,
    pub pgdata_dir: PathBuf,
}

impl DaemonHandle {
    pub fn cmd(&self) -> Command {
        let mut c = Command::new(&self.teramind);
        c.envs(self.env.iter().cloned());
        c
    }
}

/// SHA-256(pgdata_dir canonical) -> first 8 hex bytes -> "teramind_{hex}".
/// Mirrors `teramindd::app::derive_db_name`.
pub fn derive_db_name(data_dir: &Path) -> String {
    use sha2::{Digest, Sha256};
    let canonical = data_dir.canonicalize().unwrap_or_else(|_| data_dir.to_path_buf());
    let mut h = Sha256::new();
    h.update(canonical.to_string_lossy().as_bytes());
    let digest = h.finalize();
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("teramind_{hex}")
}

/// Boot a daemon in a tempdir; block until `status` reports uptime.
pub fn boot_daemon() -> DaemonHandle {
    let _ = Command::new("cargo")
        .args(["build", "--workspace"])
        .status();

    let tmp = tempfile::tempdir().unwrap();
    let teramind = cargo_bin("teramind");
    let target_dir = teramind.parent().unwrap().to_path_buf();
    let path = format!(
        "{}:{}",
        target_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let xdg_data = tmp.path().join("xdg-data");
    let pgdata_dir = xdg_data.join("teramind").join("pgdata");

    let mut env: Vec<(&'static str, String)> = vec![
        ("HOME", tmp.path().to_string_lossy().into_owned()),
        ("XDG_DATA_HOME", xdg_data.to_string_lossy().into_owned()),
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
    let pg_url = std::env::var("TERAMIND_TEST_PG_URL").ok();
    if let Some(ref url) = pg_url {
        env.push(("TERAMIND_PG_URL", url.clone()));
    }

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

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
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

    DaemonHandle {
        tmp,
        env,
        teramind,
        pg_url,
        pgdata_dir,
    }
}

/// Connect to the daemon's per-instance database. Returns None if TERAMIND_TEST_PG_URL
/// is unset (i.e. embedded PG path — those tests should skip the seeding step).
pub async fn connect_daemon_db(h: &DaemonHandle) -> Option<DbPool> {
    let url = h.pg_url.as_ref()?;
    let db_name = derive_db_name(&h.pgdata_dir);
    use sqlx::postgres::PgConnectOptions;
    let opts: PgConnectOptions = url.parse().ok()?;
    let opts = opts.database(&db_name);
    DbPool::connect(opts).await.ok()
}

pub fn stop_daemon(h: &DaemonHandle) {
    let pid_file = h.tmp.path().join("xdg-data/teramind/teramindd.pid");
    if let Ok(pid_str) = std::fs::read_to_string(&pid_file) {
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }
}
