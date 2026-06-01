//! `teramind team share-set --enable | --disable` — flips the per-project
//! team-share marker file without going through the daemon/agent.

#![cfg(unix)]
use std::process::Command;
use tempfile::tempdir;

fn cargo_bin(name: &str) -> std::path::PathBuf {
    std::env::var(format!("CARGO_BIN_EXE_{name}"))
        .map(Into::into)
        .unwrap_or_else(|_| {
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
fn share_set_enable_then_disable_flips_marker_in_cwd() {
    let _ = Command::new("cargo")
        .args(["build", "--bin", "teramind"])
        .status();
    let teramind = cargo_bin("teramind");

    let tmp = tempdir().unwrap();
    let marker_path = tmp.path().join(".teramind/team-share.toml");

    // --enable creates the marker with share=true.
    let status = Command::new(&teramind)
        .args(["team", "share-set", "--enable"])
        .current_dir(tmp.path())
        .status()
        .expect("spawn share-set --enable");
    assert!(status.success(), "team share-set --enable should succeed");
    assert!(
        marker_path.exists(),
        "marker file should be created at {}",
        marker_path.display()
    );
    let raw = std::fs::read_to_string(&marker_path).unwrap();
    assert!(
        raw.contains("share = true") || raw.contains("share=true"),
        "marker should have share=true; got: {raw}"
    );

    // --disable flips it to false.
    let status = Command::new(&teramind)
        .args(["team", "share-set", "--disable"])
        .current_dir(tmp.path())
        .status()
        .expect("spawn share-set --disable");
    assert!(status.success(), "team share-set --disable should succeed");
    let raw = std::fs::read_to_string(&marker_path).unwrap();
    assert!(
        raw.contains("share = false") || raw.contains("share=false"),
        "marker should have share=false after --disable; got: {raw}"
    );
}
