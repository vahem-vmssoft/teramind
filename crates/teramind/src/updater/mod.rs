//! Pure-Rust self-update logic. The CLI wrapper lives in
//! `commands/self_update.rs`; everything testable without an HTTP server
//! lives here so we can drive it from a tempdir-based test harness.

pub mod release_index;

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Verify a downloaded archive's SHA-256 hex digest matches `expected`.
pub fn verify_sha256(bytes: &[u8], expected_hex: &str) -> anyhow::Result<()> {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = hex::encode(hasher.finalize());
    if actual.eq_ignore_ascii_case(expected_hex) {
        Ok(())
    } else {
        anyhow::bail!("checksum mismatch: expected {expected_hex}, got {actual}")
    }
}

/// Extract a .tar.gz archive into `dest_dir`. Strips the leading path
/// component (release archives are packed as `teramind-<version>/<files>`).
pub fn extract_tarball(bytes: &[u8], dest_dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    use flate2::read::GzDecoder;
    let mut archive = tar::Archive::new(GzDecoder::new(bytes));
    let mut extracted = Vec::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        // Strip the first path component.
        let stripped: PathBuf = path.components().skip(1).collect();
        if stripped.as_os_str().is_empty() { continue; }
        let dest = dest_dir.join(&stripped);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        entry.unpack(&dest)?;
        extracted.push(dest);
    }
    Ok(extracted)
}

/// Atomically replace `target` with `staged` using rename.
/// On Unix this is atomic; on Windows we fall back to a remove-then-rename
/// which is racy in the worst case but acceptable for self-update
/// (the daemon is stopped before this runs).
pub fn atomic_swap(staged: &Path, target: &Path) -> std::io::Result<()> {
    #[cfg(unix)] { std::fs::rename(staged, target) }
    #[cfg(windows)] {
        if target.exists() {
            let backup = target.with_extension("old");
            let _ = std::fs::remove_file(&backup);
            std::fs::rename(target, &backup)?;
        }
        std::fs::rename(staged, target)
    }
}

#[cfg(test)]
mod tests_io {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;

    #[test]
    fn verify_sha256_accepts_correct_hex() {
        // sha256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        verify_sha256(b"hello", "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824")
            .expect("matches");
        verify_sha256(b"hello", "2CF24DBA5FB0A30E26E83B2AC5B9E29E1B161E5C1FA7425E73043362938B9824")
            .expect("case-insensitive");
    }

    #[test]
    fn verify_sha256_rejects_mismatch() {
        assert!(verify_sha256(b"hello", "00").is_err());
    }

    fn build_tarball(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let enc = GzEncoder::new(&mut buf, Compression::default());
            let mut tar = tar::Builder::new(enc);
            for (name, content) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_size(content.len() as u64);
                header.set_mode(0o755);
                header.set_cksum();
                tar.append_data(&mut header, name, *content).unwrap();
            }
            tar.finish().unwrap();
        }
        buf
    }

    #[test]
    fn extract_tarball_strips_leading_component() {
        let tar = build_tarball(&[
            ("teramind-0.1.0/teramind", b"BINARY"),
            ("teramind-0.1.0/teramindd", b"DAEMON"),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let extracted = extract_tarball(&tar, dir.path()).unwrap();
        assert_eq!(extracted.len(), 2);
        assert_eq!(std::fs::read(dir.path().join("teramind")).unwrap(), b"BINARY");
        assert_eq!(std::fs::read(dir.path().join("teramindd")).unwrap(), b"DAEMON");
    }

    #[test]
    fn atomic_swap_replaces_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("teramind");
        let staged = dir.path().join("teramind.new");
        std::fs::write(&target, b"OLD").unwrap();
        std::fs::write(&staged, b"NEW").unwrap();
        atomic_swap(&staged, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"NEW");
    }

    #[test]
    fn atomic_swap_creates_when_target_missing() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("teramind");
        let staged = dir.path().join("teramind.new");
        std::fs::write(&staged, b"NEW").unwrap();
        atomic_swap(&staged, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"NEW");
    }
}
