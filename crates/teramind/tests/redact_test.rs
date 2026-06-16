//! `teramind redact test` — preview redactions for sanity-check.
//!
//! Pipes an input containing an AWS-style access key id (a default redaction
//! rule) and asserts the raw secret is gone and a redacted marker is present.

#![cfg(unix)]
use std::io::Write;
use std::process::{Command, Stdio};

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
fn redact_test_masks_aws_access_key() {
    let _ = Command::new("cargo")
        .args(["build", "--bin", "teramind"])
        .status();

    let teramind = cargo_bin("teramind");
    let payload = "leak: AKIAIOSFODNN7EXAMPLE here\n";

    let mut child = Command::new(&teramind)
        .args(["redact", "test"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn teramind redact test");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait for child");
    assert!(
        out.status.success(),
        "redact test exited non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("AKIAIOSFODNN7EXAMPLE"),
        "raw AWS access key id should have been redacted; got: {stdout}"
    );
    let lower = stdout.to_lowercase();
    assert!(
        lower.contains("redacted") || lower.contains("[redacted") || stdout.contains("«redacted"),
        "expected a redacted marker in output; got: {stdout}"
    );
}
