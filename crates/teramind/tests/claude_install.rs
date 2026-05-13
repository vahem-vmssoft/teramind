#![cfg(unix)]
use std::process::Command;
use tempfile::tempdir;

fn cargo_bin(name: &str) -> std::path::PathBuf {
    std::env::var(format!("CARGO_BIN_EXE_{name}"))
        .map(Into::into)
        .unwrap_or_else(|_| {
            let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
            let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
            std::path::PathBuf::from(target).join(profile).join(name)
        })
}

#[test]
fn claude_install_uninstall_roundtrip() {
    let _ = Command::new("cargo").args(["build", "-p", "teramind-cli", "-p", "teramind-hook"]).status();

    let claude_home = tempdir().unwrap();
    let template_dir = std::env::current_dir().unwrap()
        .ancestors()
        .find(|p| p.join("plugins").join("claude").join("plugin.json").exists())
        .map(|p| p.join("plugins").join("claude"))
        .expect("could not find plugins/claude in ancestors");

    let teramind = cargo_bin("teramind");
    let env: Vec<(&str, String)> = vec![
        ("CLAUDE_HOME", claude_home.path().to_string_lossy().into_owned()),
        ("TERAMIND_PLUGIN_TEMPLATE_DIR", template_dir.to_string_lossy().into_owned()),
    ];

    // Install
    let out = Command::new(&teramind).args(["claude", "install"]).envs(env.iter().cloned()).output().unwrap();
    assert!(out.status.success(), "install failed: {}", String::from_utf8_lossy(&out.stderr));

    let manifest = claude_home.path().join("plugins/teramind/plugin.json");
    assert!(manifest.exists());
    let body = std::fs::read_to_string(&manifest).unwrap();
    assert!(!body.contains("@TERAMIND_PLUGIN_DIR@"), "placeholder left unpatched in manifest");
    assert!(body.contains(&format!("{}/plugins/teramind", claude_home.path().display())),
            "absolute plugin dir not present in manifest body");

    let hook_script = claude_home.path().join("plugins/teramind/hooks/session_start.sh");
    let body = std::fs::read_to_string(&hook_script).unwrap();
    assert!(!body.contains("@TERAMIND_HOOK_BIN@"), "placeholder left unpatched in hook script");

    // Uninstall
    let out = Command::new(&teramind).args(["claude", "uninstall"]).envs(env.iter().cloned()).output().unwrap();
    assert!(out.status.success(), "uninstall failed: {}", String::from_utf8_lossy(&out.stderr));
    assert!(!claude_home.path().join("plugins/teramind").exists(), "plugin dir not removed");
}
