//! codifier §10 — AnthropicCodifyProvider refuses to construct unless secrets.toml has
//! `network_egress = true` AND `anthropic_api_key` is set.

use std::fs;
use teramindd::services::codify::anthropic::AnthropicCodifyProvider;

fn write_secrets(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
    let path = dir.join("secrets.toml");
    fs::write(&path, body).unwrap();
    path
}

#[test]
fn rejects_when_network_egress_false_even_with_key() {
    let tmp = tempfile::tempdir().unwrap();
    let secrets = write_secrets(
        tmp.path(),
        "network_egress = false\nanthropic_api_key = \"sk-test\"\n",
    );
    let res = AnthropicCodifyProvider::try_new(&secrets, "claude-test".into());
    let err = match res {
        Ok(_) => panic!("expected Err when network_egress=false; got Ok"),
        Err(e) => e,
    };
    let msg = format!("{err}");
    assert!(
        msg.contains("network_egress"),
        "error should mention network_egress; got: {msg}"
    );
}

#[test]
fn rejects_when_network_egress_true_but_key_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let secrets = write_secrets(tmp.path(), "network_egress = true\n");
    let res = AnthropicCodifyProvider::try_new(&secrets, "claude-test".into());
    let err = match res {
        Ok(_) => panic!("expected Err when anthropic_api_key is missing; got Ok"),
        Err(e) => e,
    };
    let msg = format!("{err}");
    assert!(
        msg.contains("anthropic_api_key"),
        "error should mention anthropic_api_key; got: {msg}"
    );
}

#[test]
fn accepts_when_network_egress_true_and_key_present() {
    let tmp = tempfile::tempdir().unwrap();
    let secrets = write_secrets(
        tmp.path(),
        "network_egress = true\nanthropic_api_key = \"sk-test\"\n",
    );
    let res = AnthropicCodifyProvider::try_new(&secrets, "claude-test".into());
    assert!(
        res.is_ok(),
        "expected Ok when both gates satisfied; got Err: {:?}",
        res.err()
    );
}

#[test]
fn rejects_when_secrets_file_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let secrets = tmp.path().join("does_not_exist.toml");
    let res = AnthropicCodifyProvider::try_new(&secrets, "claude-test".into());
    assert!(
        res.is_err(),
        "expected Err when secrets.toml is absent; got Ok"
    );
}
