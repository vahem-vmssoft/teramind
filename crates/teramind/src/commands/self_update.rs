//! `teramind self-update [--check-only] [--force]`.
//!
//! Reads `TERAMIND_RELEASE_INDEX_URL` (default `https://get.teramind.dev/releases.json`),
//! finds the artifact for the current target triple, downloads it, verifies
//! its SHA-256, and atomically swaps it onto the running binary's directory.

use crate::updater::release_index::ReleaseIndex;
use crate::updater::{atomic_swap, extract_tarball, verify_sha256};
use std::path::Path;

const DEFAULT_INDEX_URL: &str = "https://get.teramind.dev/releases.json";

pub async fn run(check_only: bool, force: bool) -> anyhow::Result<()> {
    let url = std::env::var("TERAMIND_RELEASE_INDEX_URL")
        .unwrap_or_else(|_| DEFAULT_INDEX_URL.to_string());
    let current = env!("CARGO_PKG_VERSION");
    let triple = current_target_triple();

    println!("teramind self-update: current={current} target={triple}");
    println!("                     index={url}");

    let body = fetch_text(&url).await?;
    let idx: ReleaseIndex = serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("malformed release index at {url}: {e}"))?;
    let latest = &idx.latest;
    let outdated = idx.newer_than(current).is_some();

    if !outdated && !force {
        println!("teramind: already at latest ({current}); nothing to do.");
        return Ok(());
    }

    let artifact = idx.latest_artifact(&triple).ok_or_else(|| {
        anyhow::anyhow!("no release artifact for target triple {triple} in version {latest}")
    })?;
    println!("teramind: candidate {latest} for {triple}");
    println!("          url    = {}", artifact.url);
    println!("          sha256 = {}", artifact.sha256);

    if check_only {
        return Ok(());
    }

    let bytes = fetch_bytes(&artifact.url).await?;
    verify_sha256(&bytes, &artifact.sha256)?;

    let staging = tempfile::tempdir()?;
    extract_tarball(&bytes, staging.path())?;

    let current_exe = std::env::current_exe()?;
    let bin_dir = current_exe.parent()
        .ok_or_else(|| anyhow::anyhow!("current_exe() has no parent dir"))?
        .to_path_buf();
    swap_all(staging.path(), &bin_dir)?;

    println!("teramind self-update: upgraded {current} -> {latest}");
    Ok(())
}

fn swap_all(staging: &Path, bin_dir: &Path) -> anyhow::Result<()> {
    for name in ["teramind", "teramindd", "teramind-hook", "teramind-mcp"] {
        let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
        let staged = staging.join(&exe);
        if !staged.exists() {
            // Some archives may omit a binary intentionally (e.g. trimmed builds).
            // Skip rather than fail.
            continue;
        }
        let target = bin_dir.join(&exe);
        atomic_swap(&staged, &target)?;
    }
    Ok(())
}

fn current_target_triple() -> String {
    // We can't rely on `rustc -vV` at runtime. Emit the triple at build time
    // through env! when available; otherwise synthesize from compile-time cfg.
    if let Some(t) = option_env!("TERAMIND_TARGET_TRIPLE") {
        return t.to_string();
    }
    let arch = std::env::consts::ARCH; // "x86_64" / "aarch64"
    let os = std::env::consts::OS;     // "linux" / "macos" / "windows"
    match (arch, os) {
        ("aarch64", "macos")   => "aarch64-apple-darwin",
        ("x86_64",  "macos")   => "x86_64-apple-darwin",
        ("x86_64",  "linux")   => "x86_64-unknown-linux-gnu",
        ("aarch64", "linux")   => "aarch64-unknown-linux-gnu",
        ("x86_64",  "windows") => "x86_64-pc-windows-msvc",
        ("aarch64", "windows") => "aarch64-pc-windows-msvc",
        _ => "unknown-unknown-unknown",
    }.to_string()
}

async fn fetch_text(url: &str) -> anyhow::Result<String> {
    // Local-file shortcut for tests.
    if let Some(path) = url.strip_prefix("file://") {
        return Ok(std::fs::read_to_string(path)?);
    }
    let res = reqwest::Client::builder()
        .user_agent(concat!("teramind/", env!("CARGO_PKG_VERSION")))
        .build()?
        .get(url)
        .send().await?
        .error_for_status()?;
    Ok(res.text().await?)
}

async fn fetch_bytes(url: &str) -> anyhow::Result<Vec<u8>> {
    if let Some(path) = url.strip_prefix("file://") {
        return Ok(std::fs::read(path)?);
    }
    let res = reqwest::Client::builder()
        .user_agent(concat!("teramind/", env!("CARGO_PKG_VERSION")))
        .build()?
        .get(url)
        .send().await?
        .error_for_status()?;
    Ok(res.bytes().await?.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_target_triple_returns_a_known_string() {
        let t = current_target_triple();
        assert!(!t.is_empty());
        assert!(t.contains('-'), "expected triple format, got {t}");
    }
}
