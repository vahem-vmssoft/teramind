//! Drive `teramind self-update` against a tempdir-rooted release archive.
//!
//! We avoid network calls by setting TERAMIND_RELEASE_INDEX_URL to a
//! `file://` URL pointing at a hand-built releases.json that references
//! a hand-built tarball (also on disk). The updater swaps four binaries
//! into a tempdir we pretend is the bin/ dir.

use flate2::write::GzEncoder;
use flate2::Compression;
use std::path::Path;

fn build_release_tarball(dir: &Path, version: &str) -> std::path::PathBuf {
    let mut buf = Vec::new();
    {
        let enc = GzEncoder::new(&mut buf, Compression::default());
        let mut tar = tar::Builder::new(enc);
        for name in ["teramind", "teramindd", "teramind-hook", "teramind-mcp"] {
            let mut header = tar::Header::new_gnu();
            let body = format!("BIN:{name}:{version}");
            header.set_size(body.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            let path = format!("teramind-{version}/{name}");
            tar.append_data(&mut header, &path, body.as_bytes())
                .unwrap();
        }
        tar.finish().unwrap();
    }
    let tarball = dir.join(format!("teramind-{version}.tar.gz"));
    std::fs::write(&tarball, &buf).unwrap();
    tarball
}

#[tokio::test(flavor = "current_thread")]
async fn self_update_swaps_all_four_binaries() -> anyhow::Result<()> {
    use sha2::Digest;

    let dir = tempfile::tempdir()?;
    let bin_dir = dir.path().join("bin");
    std::fs::create_dir_all(&bin_dir)?;

    // Seed pretend "old" binaries so atomic_swap has something to replace.
    for name in ["teramind", "teramindd", "teramind-hook", "teramind-mcp"] {
        std::fs::write(bin_dir.join(name), "OLD").unwrap();
    }

    let tarball = build_release_tarball(dir.path(), "9.9.9");
    let bytes = std::fs::read(&tarball)?;
    let sha = hex::encode(sha2::Sha256::digest(&bytes));

    let index_path = dir.path().join("releases.json");
    let triple = std::env::consts::ARCH.to_string() + "-" + std::env::consts::OS;
    let triple = match triple.as_str() {
        "aarch64-macos" => "aarch64-apple-darwin",
        "x86_64-macos" => "x86_64-apple-darwin",
        "x86_64-linux" => "x86_64-unknown-linux-gnu",
        "aarch64-linux" => "aarch64-unknown-linux-gnu",
        "x86_64-windows" => "x86_64-pc-windows-msvc",
        "aarch64-windows" => "aarch64-pc-windows-msvc",
        _ => panic!("unsupported test target {triple}"),
    };
    let releases_json = serde_json::json!({
        "latest": "9.9.9",
        "releases": [{
            "version": "9.9.9",
            "artifacts": {
                triple: { "url": format!("file://{}", tarball.display()), "sha256": sha }
            }
        }]
    });
    std::fs::write(&index_path, serde_json::to_vec_pretty(&releases_json)?)?;

    // Redirect the swap to a temp bin_dir so the test doesn't corrupt target/debug/.
    let exe = env!("CARGO_BIN_EXE_teramind");
    let out = std::process::Command::new(exe)
        .arg("self-update")
        .env(
            "TERAMIND_RELEASE_INDEX_URL",
            format!("file://{}", index_path.display()),
        )
        .env("TERAMIND_INSTALL_ROOT", &bin_dir)
        .output()?;
    assert!(
        out.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(())
}

#[test]
fn check_only_does_not_modify_files() -> anyhow::Result<()> {
    use sha2::Digest;
    let dir = tempfile::tempdir()?;
    let tarball = build_release_tarball(dir.path(), "9.9.9");
    let sha = hex::encode(sha2::Sha256::digest(&std::fs::read(&tarball)?));
    let index_path = dir.path().join("releases.json");
    let triple = std::env::consts::ARCH.to_string() + "-" + std::env::consts::OS;
    let triple = match triple.as_str() {
        "aarch64-macos" => "aarch64-apple-darwin",
        "x86_64-macos" => "x86_64-apple-darwin",
        "x86_64-linux" => "x86_64-unknown-linux-gnu",
        "aarch64-linux" => "aarch64-unknown-linux-gnu",
        "x86_64-windows" => "x86_64-pc-windows-msvc",
        "aarch64-windows" => "aarch64-pc-windows-msvc",
        _ => panic!("unsupported test target {triple}"),
    };
    let releases_json = serde_json::json!({
        "latest": "9.9.9",
        "releases": [{
            "version": "9.9.9",
            "artifacts": {
                triple: { "url": format!("file://{}", tarball.display()), "sha256": sha }
            }
        }]
    });
    std::fs::write(&index_path, serde_json::to_vec_pretty(&releases_json)?)?;

    let exe = env!("CARGO_BIN_EXE_teramind");
    let out = std::process::Command::new(exe)
        .arg("self-update")
        .arg("--check-only")
        .env(
            "TERAMIND_RELEASE_INDEX_URL",
            format!("file://{}", index_path.display()),
        )
        .output()?;
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("candidate 9.9.9"), "stdout: {stdout}");
    Ok(())
}
