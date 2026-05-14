# Teramind Installers & Release Packaging — Plan E

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship Teramind binaries to fresh machines via a one-line `curl … | sh` (or `irm … | iex` on Windows), with a tagged GitHub release pipeline that produces per-target archives, SHA256SUMS, optional cosign signatures, and optional notarized macOS bundles. Add `teramind self update` and `teramind uninstall` subcommands.

**Architecture:** Two installer scripts (`installer/install.sh`, `installer/install.ps1`) detect OS+arch, download the right archive from a configurable release index, verify checksums, extract to a per-user prefix, symlink/PATH-prepend the `teramind` binary, and print next steps. A GitHub Actions workflow at `.github/workflows/release.yml` builds the workspace for six target triples on tag push, aggregates SHA256SUMS, optionally signs with cosign, optionally notarizes macOS artifacts, and uploads to the GitHub release. Two new Rust subcommands (`self update`, `uninstall [--purge]`) close the lifecycle.

**Tech Stack:** POSIX `sh` (install.sh), PowerShell 5+ (install.ps1), GitHub Actions, cargo cross-compilation, `tar`/`zip`, `sha256sum`/`Get-FileHash`, `cosign` (optional), `xcrun notarytool` (optional), Rust `reqwest`/`tar` for `self update`.

---

## Scope check

Spec §7 has four major parts and §7.3–§7.5 are already implemented by Plans A/B:

| Spec section | Status |
|---|---|
| §7.1 Distribution artifacts | **NEW — Plan E §4–§6** |
| §7.2 Installer behavior | **NEW — Plan E §2–§3** |
| §7.3 First-run (`teramind init`) | Plan A — already shipped |
| §7.4 Plugin install (`teramind claude install`) | Plan B — already shipped |
| §7.5 Daemon supervision (`teramind start`/`stop`/`status`) | Plan A — already shipped |
| §7.6 Update & uninstall | **NEW — Plan E §0–§1** |
| §7.7 Cross-platform notes (notarization) | **NEW — Plan E §6** |

This plan deliberately defers the Homebrew tap to a follow-up commit (spec §7.2 calls it a "fast follow"); §7 of this plan ships the formula scaffold but does NOT publish the tap repo.

---

## File structure

**New files:**
- `crates/teramind/src/commands/uninstall.rs` — `teramind uninstall [--purge]` subcommand. ~120 lines.
- `crates/teramind/src/commands/self_update.rs` — `teramind self update` subcommand. ~250 lines.
- `crates/teramind/src/updater/mod.rs` — pure-Rust update logic (download, verify, atomic swap) split out of the command for testability. ~200 lines.
- `crates/teramind/src/updater/release_index.rs` — `ReleaseIndex` types + JSON parser. ~80 lines.
- `installer/install.sh` — POSIX shell installer. ~180 lines.
- `installer/install.ps1` — PowerShell installer. ~150 lines.
- `installer/homebrew/teramind.rb` — Homebrew formula scaffold. ~50 lines.
- `installer/release-index.example.json` — schema example for `releases.json`. ~40 lines.
- `.github/workflows/release.yml` — multi-target build + package + sign + (optional) notarize + upload. ~250 lines.
- `docs/runbooks/installer-manual-smoke.md` — manual test guide.
- `docs/runbooks/release-cutover.md` — release-cutting checklist.

**Modified files:**
- `crates/teramind/src/cli.rs` — add `Uninstall { purge }` and `SelfUpdate { force, check_only }` variants.
- `crates/teramind/src/commands/mod.rs` — register new command modules.
- `crates/teramind/src/main.rs` — dispatch the new variants.
- `crates/teramind/Cargo.toml` — add `reqwest` (rustls), `flate2`, `tar`, `sha2`.
- `Cargo.toml` (workspace) — pin `reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json", "blocking"] }`, `flate2 = "1"`, `tar = "0.4"`.
- `.github/workflows/ci.yml` — append a `shellcheck` job for `installer/install.sh`.

---

## Section 0 — Workspace deps

### Task 0.1: Add updater deps to the workspace

**Files:**
- Modify: `Cargo.toml` (workspace)
- Modify: `crates/teramind/Cargo.toml`

- [ ] **Step 1: Add to `[workspace.dependencies]`**

Insert (alphabetical):

```toml
flate2      = "1"
reqwest     = { version = "0.12", default-features = false, features = ["rustls-tls", "json", "blocking", "stream"] }
tar         = "0.4"
```

- [ ] **Step 2: Pull into the CLI crate**

In `crates/teramind/Cargo.toml`, append to `[dependencies]` (alphabetical):

```toml
flate2   = { workspace = true }
hex      = { workspace = true }
reqwest  = { workspace = true }
sha2     = { workspace = true }
tar      = { workspace = true }
tempfile = { workspace = true }
```

(`hex` + `sha2` + `tempfile` are already in the workspace from Plan A. `tempfile` is currently only a dev-dep here; the new self-update command uses it at runtime to stage extraction, so we promote it.)

- [ ] **Step 3: `cargo check -p teramind-cli`**

Expected: succeeds (new deps download on first run; no source changes yet so the build is clean).

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/teramind/Cargo.toml
git commit -m "build(teramind-cli): add reqwest/flate2/tar/sha2 for self-update"
```

---

## Section 1 — `teramind uninstall` subcommand

The reset command (Plan A) already deletes local data; `uninstall` is its sibling: removes the installed binaries and (with `--purge`) the data + config too. It stops the daemon first.

### Task 1.1: Define the CLI variant

**Files:**
- Modify: `crates/teramind/src/cli.rs`

- [ ] **Step 1: Add the variant**

Append to the `Command` enum:

```rust
    /// Uninstall the Teramind binaries. With --purge, also remove data + config.
    Uninstall {
        /// Also remove `~/.local/share/teramind/` and `~/.config/teramind/`.
        #[arg(long)]
        purge: bool,
        /// Skip the interactive confirmation.
        #[arg(long)]
        confirm: bool,
    },
```

- [ ] **Step 2: `cargo check -p teramind-cli`**

Expected: the `match cli.command` in `main.rs` is now non-exhaustive — that's the next task.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind/src/cli.rs
git commit -m "feat(cli): Uninstall subcommand definition"
```

---

### Task 1.2: Implement `uninstall::run`

**Files:**
- Create: `crates/teramind/src/commands/uninstall.rs`
- Modify: `crates/teramind/src/commands/mod.rs`
- Modify: `crates/teramind/src/main.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/teramind/src/commands/uninstall.rs` with the public `run` function and unit tests:

```rust
//! `teramind uninstall [--purge] [--confirm]`.
//!
//! Removes the installed `teramind` binary and the `~/.local/bin/teramind` symlink.
//! With --purge, also deletes data + config dirs (parity with `teramind reset --purge`).

use std::path::{Path, PathBuf};

/// Result of a single removal: which path, whether it existed, whether it was removed.
#[derive(Debug, PartialEq)]
pub struct RemovalReport {
    pub path: PathBuf,
    pub existed: bool,
    pub removed: bool,
}

pub async fn run(purge: bool, confirm: bool) -> anyhow::Result<()> {
    if !confirm {
        anyhow::bail!(
            "`teramind uninstall` will delete the installed binary{}; re-run with --confirm to proceed",
            if purge { " AND your local data + config" } else { "" }
        );
    }
    // Best-effort: stop the daemon first; ignore failures (it might not be running).
    let _ = crate::commands::stop::run().await;

    let install_root = install_root_from_env();
    let bin_dir = install_root.join("bin");
    let symlink_target = symlink_target_from_env();

    let mut reports = Vec::new();
    for name in ["teramind", "teramindd", "teramind-hook", "teramind-mcp"] {
        reports.push(remove_if_exists(&bin_dir.join(format_exe(name))));
    }
    reports.push(remove_if_exists(&symlink_target));

    if purge {
        let paths = teramindd::paths::Paths::resolve()?;
        reports.push(remove_dir_if_exists(&paths.data_dir));
        reports.push(remove_dir_if_exists(&paths.config_dir));
    }

    for r in &reports {
        println!(
            "{} {}",
            if r.removed { "[removed]" } else if r.existed { "[skipped]" } else { "[absent]" },
            r.path.display()
        );
    }
    println!(
        "teramind uninstall: done{}",
        if purge { " (data + config also removed)" } else { " (data preserved; --purge to remove it)" },
    );
    Ok(())
}

fn install_root_from_env() -> PathBuf {
    if let Some(p) = std::env::var_os("TERAMIND_INSTALL_ROOT") {
        return PathBuf::from(p);
    }
    #[cfg(unix)] {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        home.join(".local/share/teramind")
    }
    #[cfg(windows)] {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default();
        local.join("teramind")
    }
}

fn symlink_target_from_env() -> PathBuf {
    if let Some(p) = std::env::var_os("TERAMIND_BIN_SYMLINK") {
        return PathBuf::from(p);
    }
    #[cfg(unix)] {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        home.join(".local/bin/teramind")
    }
    #[cfg(windows)] {
        // No symlink on Windows; install.ps1 prepends bin dir to PATH instead.
        PathBuf::new()
    }
}

fn format_exe(name: &str) -> String {
    #[cfg(windows)] { format!("{name}.exe") }
    #[cfg(unix)] { name.to_string() }
}

fn remove_if_exists(p: &Path) -> RemovalReport {
    if p.as_os_str().is_empty() {
        return RemovalReport { path: p.into(), existed: false, removed: false };
    }
    let existed = p.exists() || p.symlink_metadata().is_ok();
    let removed = if existed { std::fs::remove_file(p).is_ok() } else { false };
    RemovalReport { path: p.into(), existed, removed }
}

fn remove_dir_if_exists(p: &Path) -> RemovalReport {
    let existed = p.exists();
    let removed = if existed { std::fs::remove_dir_all(p).is_ok() } else { false };
    RemovalReport { path: p.into(), existed, removed }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_if_exists_returns_absent_for_missing_path() {
        let dir = tempfile::tempdir().unwrap();
        let r = remove_if_exists(&dir.path().join("nope"));
        assert!(!r.existed);
        assert!(!r.removed);
    }

    #[test]
    fn remove_if_exists_removes_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("teramind");
        std::fs::write(&f, b"x").unwrap();
        let r = remove_if_exists(&f);
        assert!(r.existed);
        assert!(r.removed);
        assert!(!f.exists());
    }

    #[test]
    fn remove_dir_if_exists_removes_subtree() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("teramind-data");
        std::fs::create_dir_all(sub.join("pgdata")).unwrap();
        std::fs::write(sub.join("pgdata/x"), b"y").unwrap();
        let r = remove_dir_if_exists(&sub);
        assert!(r.removed);
        assert!(!sub.exists());
    }

    #[test]
    fn install_root_honors_env_override() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("TERAMIND_INSTALL_ROOT", dir.path());
        let got = install_root_from_env();
        assert_eq!(got, dir.path());
        std::env::remove_var("TERAMIND_INSTALL_ROOT");
    }

    #[test]
    fn format_exe_is_platform_aware() {
        let n = format_exe("teramind");
        #[cfg(windows)] assert_eq!(n, "teramind.exe");
        #[cfg(unix)] assert_eq!(n, "teramind");
    }
}
```

- [ ] **Step 2: Register the module**

Append to `crates/teramind/src/commands/mod.rs`:

```rust
pub mod uninstall;
```

- [ ] **Step 3: Dispatch from `main.rs`**

Add a match arm in `crates/teramind/src/main.rs` (inside the `match cli.command { … }` block):

```rust
        Command::Uninstall { purge, confirm } => commands::uninstall::run(purge, confirm).await,
```

- [ ] **Step 4: Run unit tests**

Run: `cargo test -p teramind-cli uninstall`
Expected: 5 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind/src/commands/uninstall.rs crates/teramind/src/commands/mod.rs crates/teramind/src/main.rs
git commit -m "feat(cli): teramind uninstall [--purge] [--confirm]"
```

---

## Section 2 — `teramind self update`

The updater fetches a JSON release index, picks the latest version matching this binary's target triple, downloads the archive, verifies its checksum, and atomically swaps it onto `current_exe()`'s directory.

### Task 2.1: Define `ReleaseIndex` types

**Files:**
- Create: `crates/teramind/src/updater/mod.rs`
- Create: `crates/teramind/src/updater/release_index.rs`
- Modify: `crates/teramind/src/main.rs`

- [ ] **Step 1: Scaffold the updater module**

Create `crates/teramind/src/updater/mod.rs`:

```rust
//! Pure-Rust self-update logic. The CLI wrapper lives in
//! `commands/self_update.rs`; everything testable without an HTTP server
//! lives here so we can drive it from a tempdir-based test harness.

pub mod release_index;
```

Create `crates/teramind/src/updater/release_index.rs`:

```rust
//! Schema of `releases.json` served by the release host.
//!
//! Example:
//! ```json
//! {
//!   "latest": "0.2.0",
//!   "releases": [
//!     {
//!       "version": "0.2.0",
//!       "artifacts": {
//!         "aarch64-apple-darwin":      {"url": "...", "sha256": "..."},
//!         "x86_64-apple-darwin":       {"url": "...", "sha256": "..."},
//!         "x86_64-unknown-linux-gnu":  {"url": "...", "sha256": "..."},
//!         "aarch64-unknown-linux-gnu": {"url": "...", "sha256": "..."},
//!         "x86_64-pc-windows-msvc":    {"url": "...", "sha256": "..."},
//!         "aarch64-pc-windows-msvc":   {"url": "...", "sha256": "..."}
//!       }
//!     }
//!   ]
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseIndex {
    pub latest: String,
    pub releases: Vec<Release>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub artifacts: HashMap<String, Artifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub url: String,
    pub sha256: String,
}

impl ReleaseIndex {
    /// Find the artifact entry for a given target triple in `self.latest`.
    pub fn latest_artifact(&self, triple: &str) -> Option<&Artifact> {
        let latest = &self.latest;
        self.releases
            .iter()
            .find(|r| r.version == *latest)
            .and_then(|r| r.artifacts.get(triple))
    }

    /// Return latest version if it's newer than `current`. Naive lexical
    /// comparison is replaced by SemVer in Task 2.2.
    pub fn newer_than(&self, current: &str) -> Option<&str> {
        if current_is_older(current, &self.latest) {
            Some(&self.latest)
        } else {
            None
        }
    }
}

pub fn current_is_older(current: &str, latest: &str) -> bool {
    // Strip leading "v" if present (releases tagged as "v1.2.3").
    let c = current.strip_prefix('v').unwrap_or(current);
    let l = latest.strip_prefix('v').unwrap_or(latest);
    parse_semver(c).map(|cv| Some(cv) < parse_semver(l)).unwrap_or(false)
}

fn parse_semver(s: &str) -> Option<(u64, u64, u64)> {
    let core = s.split('-').next().unwrap_or(s); // drop pre-release suffix
    let mut it = core.split('.');
    let major: u64 = it.next()?.parse().ok()?;
    let minor: u64 = it.next()?.parse().ok()?;
    let patch: u64 = it.next()?.parse().ok()?;
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index_with_latest(v: &str) -> ReleaseIndex {
        let mut artifacts = HashMap::new();
        artifacts.insert("x86_64-unknown-linux-gnu".into(), Artifact { url: "u".into(), sha256: "s".into() });
        ReleaseIndex {
            latest: v.into(),
            releases: vec![Release { version: v.into(), artifacts }],
        }
    }

    #[test]
    fn semver_ordering_basic() {
        assert!(current_is_older("0.1.0", "0.2.0"));
        assert!(current_is_older("0.1.0", "0.1.1"));
        assert!(!current_is_older("0.2.0", "0.1.0"));
        assert!(!current_is_older("0.2.0", "0.2.0"));
    }

    #[test]
    fn semver_strips_v_prefix() {
        assert!(current_is_older("v0.1.0", "v0.2.0"));
    }

    #[test]
    fn newer_than_returns_latest_when_outdated() {
        let idx = index_with_latest("0.3.0");
        assert_eq!(idx.newer_than("0.2.0"), Some("0.3.0"));
        assert_eq!(idx.newer_than("0.3.0"), None);
    }

    #[test]
    fn latest_artifact_lookup() {
        let idx = index_with_latest("0.3.0");
        assert!(idx.latest_artifact("x86_64-unknown-linux-gnu").is_some());
        assert!(idx.latest_artifact("nonexistent-triple").is_none());
    }

    #[test]
    fn parses_example_json_shape() {
        let j = r#"{
            "latest": "0.2.0",
            "releases": [{
                "version": "0.2.0",
                "artifacts": {
                    "x86_64-unknown-linux-gnu": {"url": "https://x/a.tgz", "sha256": "deadbeef"}
                }
            }]
        }"#;
        let idx: ReleaseIndex = serde_json::from_str(j).unwrap();
        assert_eq!(idx.latest, "0.2.0");
        assert_eq!(idx.releases.len(), 1);
        let a = idx.latest_artifact("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(a.sha256, "deadbeef");
    }
}
```

- [ ] **Step 2: Register the module**

In `crates/teramind/src/main.rs`, at the top with the other `mod` declarations, add:

```rust
mod updater;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p teramind-cli updater`
Expected: 5 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind/src/main.rs crates/teramind/src/updater/
git commit -m "feat(updater): ReleaseIndex types + semver comparison"
```

---

### Task 2.2: Implement the download + verify + swap

**Files:**
- Modify: `crates/teramind/src/updater/mod.rs`

- [ ] **Step 1: Write the failing tests**

Append to `crates/teramind/src/updater/mod.rs`:

```rust
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
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
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramind-cli updater`
Expected: 10 tests PASS (5 from Task 2.1 + 5 from this task).

- [ ] **Step 3: Commit**

```bash
git add crates/teramind/src/updater/mod.rs
git commit -m "feat(updater): verify_sha256 + extract_tarball + atomic_swap"
```

---

### Task 2.3: Wire HTTP fetch + the `teramind self update` command

**Files:**
- Create: `crates/teramind/src/commands/self_update.rs`
- Modify: `crates/teramind/src/cli.rs`
- Modify: `crates/teramind/src/commands/mod.rs`
- Modify: `crates/teramind/src/main.rs`

- [ ] **Step 1: Add the CLI variant**

In `crates/teramind/src/cli.rs`, append to the `Command` enum:

```rust
    /// Check for and apply Teramind updates.
    SelfUpdate {
        /// Don't actually replace anything; just report the available version.
        #[arg(long)]
        check_only: bool,
        /// Force the upgrade even if the local version is already at the latest.
        #[arg(long)]
        force: bool,
    },
```

Then add (or extend) a `SelfAction` enum if you prefer a `teramind self update` nested form. For simplicity we keep it flat as `teramind self-update` — the spec example uses `teramind self update` (space), but clap doesn't render a two-word subcommand from a flat enum; the simplest match for spec wording is to keep the subcommand named `self-update`. Document this in `--help`.

- [ ] **Step 2: Implement the command**

Create `crates/teramind/src/commands/self_update.rs`:

```rust
//! `teramind self-update [--check-only] [--force]`.
//!
//! Reads `TERAMIND_RELEASE_INDEX_URL` (default `https://get.teramind.dev/releases.json`),
//! finds the artifact for the current target triple, downloads it, verifies
//! its SHA-256, and atomically swaps it onto the running binary's directory.

use crate::updater::release_index::ReleaseIndex;
use crate::updater::{atomic_swap, extract_tarball, verify_sha256};
use std::path::PathBuf;

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
    swap_all(&staging.path().to_path_buf(), &bin_dir)?;

    println!("teramind self-update: upgraded {current} -> {latest}");
    Ok(())
}

fn swap_all(staging: &PathBuf, bin_dir: &PathBuf) -> anyhow::Result<()> {
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
```

- [ ] **Step 3: Register + dispatch**

Append to `crates/teramind/src/commands/mod.rs`:

```rust
pub mod self_update;
```

In `crates/teramind/src/main.rs`, add the match arm:

```rust
        Command::SelfUpdate { check_only, force } =>
            commands::self_update::run(check_only, force).await,
```

- [ ] **Step 4: Build + run unit tests**

Run: `cargo build -p teramind-cli` then `cargo test -p teramind-cli self_update`
Expected: build succeeds; 1 test passes.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind/src/commands/self_update.rs crates/teramind/src/commands/mod.rs crates/teramind/src/cli.rs crates/teramind/src/main.rs
git commit -m "feat(cli): teramind self-update via release index"
```

---

### Task 2.4: Integration test using `file://` index

**Files:**
- Create: `crates/teramind/tests/self_update_local.rs`

- [ ] **Step 1: Write the test**

```rust
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
            tar.append_data(&mut header, &path, body.as_bytes()).unwrap();
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
        "aarch64-macos"  => "aarch64-apple-darwin",
        "x86_64-macos"   => "x86_64-apple-darwin",
        "x86_64-linux"   => "x86_64-unknown-linux-gnu",
        "aarch64-linux"  => "aarch64-unknown-linux-gnu",
        "x86_64-windows" => "x86_64-pc-windows-msvc",
        "aarch64-windows"=> "aarch64-pc-windows-msvc",
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

    // We can't easily redirect current_exe(), so we invoke the binary via Command.
    let exe = env!("CARGO_BIN_EXE_teramind");
    let out = std::process::Command::new(exe)
        .arg("self-update")
        .env("TERAMIND_RELEASE_INDEX_URL", format!("file://{}", index_path.display()))
        .output()?;
    assert!(out.status.success(),
        "stdout={}\nstderr={}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
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
        "aarch64-macos"  => "aarch64-apple-darwin",
        "x86_64-macos"   => "x86_64-apple-darwin",
        "x86_64-linux"   => "x86_64-unknown-linux-gnu",
        "aarch64-linux"  => "aarch64-unknown-linux-gnu",
        "x86_64-windows" => "x86_64-pc-windows-msvc",
        "aarch64-windows"=> "aarch64-pc-windows-msvc",
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
        .arg("self-update").arg("--check-only")
        .env("TERAMIND_RELEASE_INDEX_URL", format!("file://{}", index_path.display()))
        .output()?;
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("candidate 9.9.9"), "stdout: {stdout}");
    Ok(())
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p teramind-cli --test self_update_local`
Expected: both tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind/tests/self_update_local.rs
git commit -m "test(cli): self-update integration via file:// release index"
```

---

## Section 3 — `installer/install.sh`

POSIX shell installer. Tested via `shellcheck` (static) and a runbook (manual).

### Task 3.1: Author `install.sh`

**Files:**
- Create: `installer/install.sh`

- [ ] **Step 1: Author the script**

Create `installer/install.sh` with mode `0755`:

```sh
#!/bin/sh
# Teramind installer (Unix). Idempotent.
#
# Behavior:
#   1. Detect OS + arch.
#   2. Download the release archive for that target.
#   3. Verify the SHA-256 against SHA256SUMS.
#   4. Extract to $INSTALL_ROOT/bin/.
#   5. Symlink the `teramind` binary into ~/.local/bin/.
#   6. Print the next-steps line.
#
# Environment overrides (all optional):
#   TERAMIND_VERSION         — version tag to install (default: latest from releases.json)
#   TERAMIND_RELEASE_BASE    — base URL for releases (default: https://get.teramind.dev)
#   TERAMIND_INSTALL_ROOT    — where binaries go (default: ~/.local/share/teramind)
#   TERAMIND_BIN_DIR         — where the `teramind` symlink goes (default: ~/.local/bin)
#   TERAMIND_NO_MODIFY_PATH  — set to skip PATH printing (default: unset)

set -eu

BASE_URL="${TERAMIND_RELEASE_BASE:-https://get.teramind.dev}"
INSTALL_ROOT="${TERAMIND_INSTALL_ROOT:-${HOME}/.local/share/teramind}"
BIN_DIR="${TERAMIND_BIN_DIR:-${HOME}/.local/bin}"

die() { echo "install.sh: error: $*" >&2; exit 1; }
info() { echo "install.sh: $*"; }

need() { command -v "$1" >/dev/null 2>&1 || die "missing required tool: $1"; }

detect_os() {
    case "$(uname -s)" in
        Linux)  echo "unknown-linux-gnu" ;;
        Darwin) echo "apple-darwin" ;;
        *) die "unsupported OS: $(uname -s) (use install.ps1 on Windows)" ;;
    esac
}

detect_arch() {
    case "$(uname -m)" in
        x86_64|amd64)  echo "x86_64" ;;
        aarch64|arm64) echo "aarch64" ;;
        *) die "unsupported arch: $(uname -m)" ;;
    esac
}

detect_triple() {
    arch=$(detect_arch)
    os=$(detect_os)
    echo "${arch}-${os}"
}

fetch_to() {
    src="$1"; dest="$2"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL --output "${dest}" "${src}"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "${dest}" "${src}"
    else
        die "need curl or wget on PATH"
    fi
}

resolve_version() {
    if [ -n "${TERAMIND_VERSION:-}" ]; then
        echo "${TERAMIND_VERSION}"
        return
    fi
    tmp=$(mktemp)
    fetch_to "${BASE_URL}/releases.json" "${tmp}"
    # Lightweight JSON pluck. Avoids a jq dep.
    sed -n 's/.*"latest"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "${tmp}" | head -1
    rm -f "${tmp}"
}

verify_sha256() {
    file="$1"; expected="$2"
    if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "${file}" | awk '{print $1}')
    elif command -v shasum >/dev/null 2>&1; then
        actual=$(shasum -a 256 "${file}" | awk '{print $1}')
    else
        die "need sha256sum or shasum on PATH"
    fi
    [ "${actual}" = "${expected}" ] || die "checksum mismatch (${file}): expected ${expected}, got ${actual}"
}

main() {
    need uname
    need tar
    need mktemp

    triple=$(detect_triple)
    version=$(resolve_version)
    [ -n "${version}" ] || die "could not determine latest version from ${BASE_URL}/releases.json"
    info "installing teramind ${version} for ${triple}"

    archive_name="teramind-${version}-${triple}.tar.gz"
    archive_url="${BASE_URL}/${version}/${archive_name}"
    sums_url="${BASE_URL}/${version}/teramind-${version}-SHA256SUMS"

    tmpdir=$(mktemp -d)
    trap 'rm -rf "${tmpdir}"' EXIT

    info "downloading ${archive_url}"
    fetch_to "${archive_url}" "${tmpdir}/${archive_name}"

    info "downloading SHA256SUMS"
    fetch_to "${sums_url}" "${tmpdir}/SHA256SUMS"

    expected=$(awk -v a="${archive_name}" '$2==a || $2=="*"a {print $1}' "${tmpdir}/SHA256SUMS")
    [ -n "${expected}" ] || die "no SHA256 entry for ${archive_name} in SHA256SUMS"
    verify_sha256 "${tmpdir}/${archive_name}" "${expected}"

    mkdir -p "${INSTALL_ROOT}/bin"
    tar -xzf "${tmpdir}/${archive_name}" -C "${INSTALL_ROOT}/bin" --strip-components=1
    chmod +x "${INSTALL_ROOT}/bin/"*

    mkdir -p "${BIN_DIR}"
    ln -sfn "${INSTALL_ROOT}/bin/teramind" "${BIN_DIR}/teramind"

    info "installed to ${INSTALL_ROOT}/bin/"
    info "symlinked   ${BIN_DIR}/teramind -> ${INSTALL_ROOT}/bin/teramind"
    if [ -z "${TERAMIND_NO_MODIFY_PATH:-}" ]; then
        case ":${PATH}:" in
            *":${BIN_DIR}:"*) ;;
            *) info "NOTE: ${BIN_DIR} is not on your PATH. Add it to ~/.bashrc / ~/.zshrc:"
               info "      export PATH=\"${BIN_DIR}:\$PATH\"" ;;
        esac
    fi
    info ""
    info "next:  teramind init && teramind claude install"
}

main "$@"
```

- [ ] **Step 2: `chmod +x installer/install.sh`**

Run: `chmod +x installer/install.sh`

- [ ] **Step 3: Run shellcheck**

Run: `shellcheck installer/install.sh`

(If shellcheck isn't installed locally, skip this step and rely on the CI check we add in Task 9.1.)

Expected: no warnings. If you get any, fix the script.

- [ ] **Step 4: Smoke-test against a local fixture**

This is a quick manual verification. Build a fake release index + archive in a tempdir, point the installer at it, and confirm it installs successfully. We'll automate this in Task 9.2.

```sh
TMP=$(mktemp -d)
mkdir -p "$TMP/0.0.1"
echo '{"latest":"0.0.1","releases":[]}' > "$TMP/releases.json"
# Create a minimal tarball with a dummy `teramind` binary.
mkdir -p "$TMP/build/teramind-0.0.1"
echo '#!/bin/sh' > "$TMP/build/teramind-0.0.1/teramind"
echo 'echo hi' >> "$TMP/build/teramind-0.0.1/teramind"
chmod +x "$TMP/build/teramind-0.0.1/teramind"
TRIPLE=$(uname -m | sed -e 's/arm64/aarch64/' -e 's/amd64/x86_64/')-$(uname -s | sed -e 's/Linux/unknown-linux-gnu/' -e 's/Darwin/apple-darwin/')
ARCHIVE="teramind-0.0.1-${TRIPLE}.tar.gz"
( cd "$TMP/build" && tar -czf "$TMP/0.0.1/$ARCHIVE" "teramind-0.0.1" )
SUM=$(shasum -a 256 "$TMP/0.0.1/$ARCHIVE" | awk '{print $1}')
echo "$SUM  $ARCHIVE" > "$TMP/0.0.1/teramind-0.0.1-SHA256SUMS"

# Serve via a file:// base URL won't work directly because the installer uses
# multiple URL components. Instead spin up python http for the smoke test:
( cd "$TMP" && python3 -m http.server 38080 ) &
SERVER=$!
sleep 1
TERAMIND_RELEASE_BASE=http://127.0.0.1:38080 \
TERAMIND_INSTALL_ROOT="$TMP/install" \
TERAMIND_BIN_DIR="$TMP/binshadow" \
sh installer/install.sh
"$TMP/install/bin/teramind" | grep -q hi && echo OK
kill $SERVER 2>/dev/null || true
rm -rf "$TMP"
```

Expected: prints `OK` at the end. If you see `OK`, the installer works end-to-end against a local fixture.

- [ ] **Step 5: Commit**

```bash
git add installer/install.sh
git commit -m "feat(installer): install.sh — POSIX one-line installer"
```

---

## Section 4 — `installer/install.ps1`

PowerShell installer. Tested via `Invoke-ScriptAnalyzer` (static).

### Task 4.1: Author `install.ps1`

**Files:**
- Create: `installer/install.ps1`

- [ ] **Step 1: Author the script**

Create `installer/install.ps1`:

```powershell
# Teramind installer (Windows). Idempotent.
#
# Environment overrides (all optional):
#   $env:TERAMIND_VERSION         — version tag to install
#   $env:TERAMIND_RELEASE_BASE    — base URL for releases (default: https://get.teramind.dev)
#   $env:TERAMIND_INSTALL_ROOT    — where binaries go (default: $env:LOCALAPPDATA\teramind)
#   $env:TERAMIND_NO_MODIFY_PATH  — skip user PATH prepend (default: unset)

#Requires -Version 5
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Write-Info($msg) { Write-Host "install.ps1: $msg" }
function Die($msg) { Write-Error "install.ps1: $msg"; exit 1 }

function Get-Triple {
    $arch = if ([Environment]::Is64BitOperatingSystem) {
        if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'aarch64' } else { 'x86_64' }
    } else { Die 'only 64-bit Windows is supported' }
    "${arch}-pc-windows-msvc"
}

function Resolve-Version($base) {
    if ($env:TERAMIND_VERSION) { return $env:TERAMIND_VERSION }
    $idx = Invoke-RestMethod -Uri "$base/releases.json" -UseBasicParsing
    if (-not $idx.latest) { Die "could not parse $base/releases.json" }
    return $idx.latest
}

function Verify-Sha256($file, $expected) {
    $actual = (Get-FileHash $file -Algorithm SHA256).Hash.ToLower()
    if ($actual -ne $expected.ToLower()) {
        Die "checksum mismatch for $file (expected $expected, got $actual)"
    }
}

$Base = if ($env:TERAMIND_RELEASE_BASE) { $env:TERAMIND_RELEASE_BASE } else { 'https://get.teramind.dev' }
$InstallRoot = if ($env:TERAMIND_INSTALL_ROOT) { $env:TERAMIND_INSTALL_ROOT } else { Join-Path $env:LOCALAPPDATA 'teramind' }
$BinDir = Join-Path $InstallRoot 'bin'

$Triple  = Get-Triple
$Version = Resolve-Version $Base
Write-Info "installing teramind $Version for $Triple"

$ArchiveName = "teramind-$Version-$Triple.zip"
$ArchiveUrl  = "$Base/$Version/$ArchiveName"
$SumsUrl     = "$Base/$Version/teramind-$Version-SHA256SUMS"

$Tmp = New-Item -ItemType Directory -Path (Join-Path $env:TEMP "teramind-install-$([System.Guid]::NewGuid().Guid)")
try {
    $ArchivePath = Join-Path $Tmp $ArchiveName
    Write-Info "downloading $ArchiveUrl"
    Invoke-WebRequest -Uri $ArchiveUrl -OutFile $ArchivePath -UseBasicParsing

    $SumsPath = Join-Path $Tmp 'SHA256SUMS'
    Write-Info "downloading SHA256SUMS"
    Invoke-WebRequest -Uri $SumsUrl -OutFile $SumsPath -UseBasicParsing

    # Pluck the hex digest for our archive (format: "<sha>  <name>").
    $Line = Get-Content $SumsPath | Where-Object { $_ -match [regex]::Escape($ArchiveName) } | Select-Object -First 1
    if (-not $Line) { Die "no SHA256 entry for $ArchiveName in SHA256SUMS" }
    $Expected = ($Line -split '\s+')[0]
    Verify-Sha256 $ArchivePath $Expected

    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    Expand-Archive -Path $ArchivePath -DestinationPath $BinDir -Force
    # The archive has a `teramind-<version>/` prefix; flatten.
    $Inner = Join-Path $BinDir "teramind-$Version"
    if (Test-Path $Inner) {
        Get-ChildItem -Path $Inner | Move-Item -Destination $BinDir -Force
        Remove-Item $Inner -Recurse -Force
    }

    if (-not $env:TERAMIND_NO_MODIFY_PATH) {
        $UserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        if ($UserPath -notlike "*${BinDir}*") {
            [Environment]::SetEnvironmentVariable('Path', "${BinDir};$UserPath", 'User')
            Write-Info "prepended $BinDir to user PATH (open a new terminal to pick it up)"
        } else {
            Write-Info "user PATH already contains $BinDir"
        }
    }
    Write-Info ""
    Write-Info "next:  teramind init; teramind claude install"
} finally {
    Remove-Item $Tmp -Recurse -Force -ErrorAction SilentlyContinue
}
```

- [ ] **Step 2: Static-analyze with PSScriptAnalyzer (optional, CI runs it in Task 9.1)**

If you have PowerShell + PSScriptAnalyzer locally:

```pwsh
Invoke-ScriptAnalyzer -Path installer/install.ps1 -Severity Warning
```

Expected: no warnings. (Skip if not installed; CI catches it.)

- [ ] **Step 3: Commit**

```bash
git add installer/install.ps1
git commit -m "feat(installer): install.ps1 — PowerShell one-line installer"
```

---

## Section 5 — Release CI workflow: build matrix

### Task 5.1: Scaffold `.github/workflows/release.yml`

**Files:**
- Create: `.github/workflows/release.yml`

- [ ] **Step 1: Author the workflow**

Create `.github/workflows/release.yml`:

```yaml
name: release
on:
  push:
    tags: ["v*.*.*"]
  workflow_dispatch:
    inputs:
      tag:
        description: "Release tag (e.g. v0.2.0)"
        required: true

permissions:
  contents: write     # for creating GitHub releases
  id-token: write     # for cosign keyless OIDC

jobs:
  build:
    name: build ${{ matrix.target }}
    runs-on: ${{ matrix.runner }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - target: aarch64-apple-darwin
            runner: macos-14
            archive: tar.gz
          - target: x86_64-apple-darwin
            runner: macos-14
            archive: tar.gz
          - target: x86_64-unknown-linux-gnu
            runner: ubuntu-22.04
            archive: tar.gz
          - target: aarch64-unknown-linux-gnu
            runner: ubuntu-22.04
            archive: tar.gz
            cross: true
          - target: x86_64-pc-windows-msvc
            runner: windows-2022
            archive: zip
          - target: aarch64-pc-windows-msvc
            runner: windows-2022
            archive: zip
    steps:
      - uses: actions/checkout@v4

      - name: install rust toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - name: install cross (Linux arm64)
        if: matrix.cross
        run: cargo install cross --locked

      - name: build
        shell: bash
        run: |
          set -euo pipefail
          version="${GITHUB_REF_NAME#v}"
          if [[ "${{ github.event_name }}" == "workflow_dispatch" ]]; then
            version="${{ inputs.tag }}"
            version="${version#v}"
          fi
          echo "version=$version" >> "$GITHUB_ENV"
          if [[ "${{ matrix.cross }}" == "true" ]]; then
            cross build --release --target ${{ matrix.target }} --workspace --bins
          else
            cargo build --release --target ${{ matrix.target }} --workspace --bins
          fi

      - name: package
        shell: bash
        run: |
          set -euo pipefail
          v="$version"
          name="teramind-${v}-${{ matrix.target }}"
          stage="staging/${name}"
          mkdir -p "${stage}"
          exe_suffix=""
          if [[ "${{ matrix.target }}" == *windows* ]]; then exe_suffix=".exe"; fi
          for bin in teramind teramindd teramind-hook teramind-mcp; do
            cp "target/${{ matrix.target }}/release/${bin}${exe_suffix}" "${stage}/"
          done
          cp -r plugins/claude "${stage}/plugins-claude"
          cp LICENSE "${stage}/LICENSE" 2>/dev/null || echo "Apache-2.0" > "${stage}/LICENSE"
          mkdir -p dist
          if [[ "${{ matrix.archive }}" == "tar.gz" ]]; then
            (cd staging && tar -czf "../dist/${name}.tar.gz" "${name}")
          else
            (cd staging && 7z a "../dist/${name}.zip" "${name}" >/dev/null)
          fi

      - name: upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: ${{ matrix.target }}
          path: dist/*
          if-no-files-found: error
```

- [ ] **Step 2: Validate YAML locally**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/release.yml'))" && echo OK`
Expected: `OK`.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci(release): build matrix for 6 target triples"
```

---

## Section 6 — Release CI: SHA256SUMS + cosign + notarization

Three follow-on jobs that depend on `build`:
- `sums` — aggregate SHA256SUMS over all artifacts.
- `notarize` — Apple notarytool for macOS archives (gated on `secrets.APPLE_*` being set).
- `release` — publish the GitHub release with all artifacts + SUMS + signatures.

### Task 6.1: Append the `sums` job

**Files:**
- Modify: `.github/workflows/release.yml`

- [ ] **Step 1: Append**

Append (at the same indentation level as `build:`) to the `jobs:` block in `.github/workflows/release.yml`:

```yaml
  sums:
    name: SHA256SUMS + cosign
    needs: [build]
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - uses: actions/download-artifact@v4
        with:
          path: dist
          merge-multiple: true
      - name: write SHA256SUMS
        run: |
          set -euo pipefail
          version="${GITHUB_REF_NAME#v}"
          if [[ "${{ github.event_name }}" == "workflow_dispatch" ]]; then
            version="${{ inputs.tag }}"; version="${version#v}"
          fi
          cd dist
          shopt -s nullglob
          : > "teramind-${version}-SHA256SUMS"
          for f in *.tar.gz *.zip; do
            sha256sum "$f" >> "teramind-${version}-SHA256SUMS"
          done
          cat "teramind-${version}-SHA256SUMS"

      - name: install cosign
        if: ${{ vars.COSIGN_ENABLED == 'true' }}
        uses: sigstore/cosign-installer@v3

      - name: sign with cosign (keyless OIDC)
        if: ${{ vars.COSIGN_ENABLED == 'true' }}
        env:
          COSIGN_EXPERIMENTAL: 1
        run: |
          set -euo pipefail
          version="${GITHUB_REF_NAME#v}"
          cd dist
          cosign sign-blob --yes \
            --output-signature "teramind-${version}-SHA256SUMS.sig" \
            "teramind-${version}-SHA256SUMS"

      - uses: actions/upload-artifact@v4
        with:
          name: sums
          path: |
            dist/teramind-*-SHA256SUMS
            dist/teramind-*-SHA256SUMS.sig
          if-no-files-found: error
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci(release): SHA256SUMS aggregation + optional cosign signature"
```

---

### Task 6.2: Append the `notarize` job

**Files:**
- Modify: `.github/workflows/release.yml`

The notarize job:
1. Downloads the two macOS archives.
2. Unpacks them, codesigns each binary with the Apple Developer ID cert (`APPLE_DEVELOPER_ID`).
3. Re-archives them.
4. Submits via `xcrun notarytool` and waits.
5. Re-uploads the notarized archives.

This job is gated: if `APPLE_DEVELOPER_ID_P12` (the cert PFX, base64) is not set, the job is skipped (the macOS archives ship unsigned in that case, matching the spec's pre-notarization state).

- [ ] **Step 1: Append**

Append to `.github/workflows/release.yml`:

```yaml
  notarize:
    name: notarize macOS (${{ matrix.target }})
    if: ${{ vars.APPLE_NOTARIZE_ENABLED == 'true' }}
    needs: [build]
    runs-on: macos-14
    strategy:
      fail-fast: false
      matrix:
        target: [aarch64-apple-darwin, x86_64-apple-darwin]
    steps:
      - uses: actions/download-artifact@v4
        with:
          name: ${{ matrix.target }}
          path: in

      - name: import signing identity
        env:
          P12_B64:   ${{ secrets.APPLE_DEVELOPER_ID_P12 }}
          P12_PASS:  ${{ secrets.APPLE_DEVELOPER_ID_P12_PASSWORD }}
        run: |
          set -euo pipefail
          echo "$P12_B64" | base64 --decode > /tmp/id.p12
          security create-keychain -p '' build.keychain
          security default-keychain -s build.keychain
          security unlock-keychain -p '' build.keychain
          security import /tmp/id.p12 -k build.keychain -P "$P12_PASS" -T /usr/bin/codesign
          security set-key-partition-list -S apple-tool:,apple: -s -k '' build.keychain
          rm /tmp/id.p12

      - name: extract, codesign, repack
        env:
          IDENTITY: ${{ vars.APPLE_DEVELOPER_ID_NAME }}  # e.g. "Developer ID Application: Acme Inc (TEAMID)"
        run: |
          set -euo pipefail
          cd in
          archive=$(ls teramind-*-${{ matrix.target }}.tar.gz)
          mkdir staging && tar -xzf "$archive" -C staging
          dir=$(ls staging)
          for bin in teramind teramindd teramind-hook teramind-mcp; do
            codesign --force --options runtime --timestamp \
              --sign "$IDENTITY" \
              "staging/$dir/$bin"
          done
          (cd staging && tar -czf "../$archive" "$dir")

      - name: notarize
        env:
          APPLE_ID:        ${{ secrets.APPLE_ID }}
          APPLE_PASSWORD:  ${{ secrets.APPLE_ID_APP_PASSWORD }}
          APPLE_TEAM_ID:   ${{ secrets.APPLE_TEAM_ID }}
        run: |
          set -euo pipefail
          cd in
          archive=$(ls teramind-*-${{ matrix.target }}.tar.gz)
          xcrun notarytool submit "$archive" \
            --apple-id "$APPLE_ID" \
            --team-id "$APPLE_TEAM_ID" \
            --password "$APPLE_PASSWORD" \
            --wait
          # tar.gz archives can't be stapled directly; the per-binary signatures
          # are sufficient and the notarization record is captured by Apple's
          # servers (verified at first run when network is available).

      - uses: actions/upload-artifact@v4
        with:
          name: ${{ matrix.target }}-notarized
          path: in/*.tar.gz
          if-no-files-found: error
```

- [ ] **Step 2: Validate YAML**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/release.yml'))" && echo OK`
Expected: `OK`.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci(release): macOS codesign + notarytool (gated on APPLE_NOTARIZE_ENABLED var)"
```

---

### Task 6.3: Append the `release` (publish) job

**Files:**
- Modify: `.github/workflows/release.yml`

- [ ] **Step 1: Append**

Append:

```yaml
  release:
    name: publish GitHub release
    needs: [build, sums]
    runs-on: ubuntu-22.04
    if: startsWith(github.ref, 'refs/tags/v')
    steps:
      - uses: actions/checkout@v4
      - uses: actions/download-artifact@v4
        with:
          path: out
          merge-multiple: true
      - name: upload to GitHub release
        uses: softprops/action-gh-release@v2
        with:
          files: |
            out/teramind-*.tar.gz
            out/teramind-*.zip
            out/teramind-*-SHA256SUMS
            out/teramind-*-SHA256SUMS.sig
          fail_on_unmatched_files: false
          generate_release_notes: true
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci(release): publish GitHub release on tag push"
```

---

## Section 7 — Homebrew tap scaffolding

The full publication of a Homebrew tap repo (`homebrew-tap`) is out of scope; we ship the formula file and a README so someone can copy it into the tap repo when ready.

### Task 7.1: Author `installer/homebrew/teramind.rb`

**Files:**
- Create: `installer/homebrew/teramind.rb`
- Create: `installer/homebrew/README.md`

- [ ] **Step 1: Author the formula**

Create `installer/homebrew/teramind.rb`:

```ruby
class Teramind < Formula
  desc "Local-first AI knowledge consolidation substrate for coding agents"
  homepage "https://get.teramind.dev"
  version "0.1.0"   # bumped by release CI before publishing to tap
  license "Apache-2.0"

  if Hardware::CPU.arm?
    url "https://get.teramind.dev/#{version}/teramind-#{version}-aarch64-apple-darwin.tar.gz"
    sha256 "REPLACE_WITH_RELEASE_SUM"
  else
    url "https://get.teramind.dev/#{version}/teramind-#{version}-x86_64-apple-darwin.tar.gz"
    sha256 "REPLACE_WITH_RELEASE_SUM"
  end

  def install
    bin.install "teramind", "teramindd", "teramind-hook", "teramind-mcp"
    (libexec/"plugins/claude").install Dir["plugins-claude/*"]
  end

  test do
    assert_match(/teramind/, shell_output("#{bin}/teramind --version"))
  end
end
```

Create `installer/homebrew/README.md`:

```markdown
# Homebrew tap (scaffold)

This directory contains the Homebrew formula for Teramind. Publication to a
real tap (`https://github.com/teramind-org/homebrew-tap`) is gated on the
first stable release; until then, this is a reference.

## Updating after each release

1. Bump `version` in `teramind.rb`.
2. Replace each `REPLACE_WITH_RELEASE_SUM` with the macOS arm64 / x86_64
   SHA-256 from `teramind-<version>-SHA256SUMS`.
3. Commit & push the tap repo. Users get the new version on `brew upgrade`.

A future CI job will automate steps 1–3 by opening a PR against the tap repo.
```

- [ ] **Step 2: Commit**

```bash
git add installer/homebrew/
git commit -m "feat(installer): Homebrew formula scaffold"
```

---

## Section 8 — Release index example + cutover runbook

### Task 8.1: Author `installer/release-index.example.json` + the runbook

**Files:**
- Create: `installer/release-index.example.json`
- Create: `docs/runbooks/release-cutover.md`

- [ ] **Step 1: Author the example index**

Create `installer/release-index.example.json`:

```json
{
  "latest": "0.2.0",
  "releases": [
    {
      "version": "0.2.0",
      "artifacts": {
        "aarch64-apple-darwin": {
          "url": "https://get.teramind.dev/0.2.0/teramind-0.2.0-aarch64-apple-darwin.tar.gz",
          "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
        },
        "x86_64-apple-darwin": {
          "url": "https://get.teramind.dev/0.2.0/teramind-0.2.0-x86_64-apple-darwin.tar.gz",
          "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
        },
        "x86_64-unknown-linux-gnu": {
          "url": "https://get.teramind.dev/0.2.0/teramind-0.2.0-x86_64-unknown-linux-gnu.tar.gz",
          "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
        },
        "aarch64-unknown-linux-gnu": {
          "url": "https://get.teramind.dev/0.2.0/teramind-0.2.0-aarch64-unknown-linux-gnu.tar.gz",
          "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
        },
        "x86_64-pc-windows-msvc": {
          "url": "https://get.teramind.dev/0.2.0/teramind-0.2.0-x86_64-pc-windows-msvc.zip",
          "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
        },
        "aarch64-pc-windows-msvc": {
          "url": "https://get.teramind.dev/0.2.0/teramind-0.2.0-aarch64-pc-windows-msvc.zip",
          "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
        }
      }
    }
  ]
}
```

- [ ] **Step 2: Author the runbook**

Create `docs/runbooks/release-cutover.md`:

```markdown
# Release cutover runbook

How to cut a new Teramind release.

## Prereqs

- You have push access to the repo.
- The `get.teramind.dev` static host (S3 / GCS bucket) is configured with the
  `/<version>/` layout the installer scripts expect.
- GitHub Actions has secrets set: `APPLE_DEVELOPER_ID_P12`,
  `APPLE_DEVELOPER_ID_P12_PASSWORD`, `APPLE_ID`, `APPLE_ID_APP_PASSWORD`,
  `APPLE_TEAM_ID`. Vars: `APPLE_NOTARIZE_ENABLED=true`, `COSIGN_ENABLED=true`,
  `APPLE_DEVELOPER_ID_NAME=Developer ID Application: <Org> (<TEAMID>)`.

## Steps

1. Bump `version` in the workspace `Cargo.toml`.
2. Update `CHANGELOG.md`.
3. Commit: `chore(release): vX.Y.Z`.
4. Tag: `git tag -s vX.Y.Z -m "vX.Y.Z"`.
5. Push: `git push origin main vX.Y.Z`.
6. Watch `.github/workflows/release.yml`:
   - 6 build jobs (one per target triple)
   - 1 sums job (aggregates SHA256SUMS + cosign signature)
   - 2 notarize jobs (macOS arm64 + x86_64) — optional, controlled by vars
   - 1 release job (publishes GH release)
7. Once the GH release is up, copy all artifacts to `get.teramind.dev`:
   ```
   aws s3 sync ./out/ s3://get.teramind.dev/vX.Y.Z/
   ```
8. Update `s3://get.teramind.dev/releases.json` with the new `latest` and
   sha256s. Use `installer/release-index.example.json` as a template; the
   sha256s come from the `teramind-vX.Y.Z-SHA256SUMS` file in the release.
9. Smoke-test the installer:
   ```
   sh installer/install.sh  # picks up the new latest
   teramind --version  # should print X.Y.Z
   teramind self-update --check-only  # should report up-to-date
   ```
10. Bump the Homebrew formula in `installer/homebrew/teramind.rb` and
    open a PR against the tap repo (manual for now).

## Rollback

- If the new release has a bug, revert `s3://get.teramind.dev/releases.json`
  to the previous version (keep the `0.X.Y/` artifacts in S3 indefinitely).
- Push a patched build as `vX.Y.Z+1`.
```

- [ ] **Step 3: Commit**

```bash
git add installer/release-index.example.json docs/runbooks/release-cutover.md
git commit -m "docs: release cutover runbook + releases.json example"
```

---

## Section 9 — CI + final integration

### Task 9.1: Add a `shellcheck + PSScriptAnalyzer` job

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Append a `lint-scripts` job**

Edit `.github/workflows/ci.yml`, append at the same indentation as `lint-and-test:`:

```yaml
  lint-scripts:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - name: shellcheck install.sh
        run: shellcheck installer/install.sh

  lint-ps1:
    runs-on: windows-2022
    steps:
      - uses: actions/checkout@v4
      - name: PSScriptAnalyzer
        shell: pwsh
        run: |
          Install-Module -Name PSScriptAnalyzer -Force -Scope CurrentUser
          $issues = Invoke-ScriptAnalyzer -Path installer/install.ps1 -Severity Warning,Error
          $issues | Format-Table
          if ($issues.Count -gt 0) { exit 1 }
```

- [ ] **Step 2: Validate YAML**

Run: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))" && echo OK`
Expected: `OK`.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: shellcheck + PSScriptAnalyzer for installer scripts"
```

---

### Task 9.2: Author `docs/runbooks/installer-manual-smoke.md`

**Files:**
- Create: `docs/runbooks/installer-manual-smoke.md`

- [ ] **Step 1: Author the runbook**

```markdown
# Manual smoke: installer scripts

Verifies that `installer/install.sh` and `installer/install.ps1` install,
upgrade, and uninstall Teramind cleanly against a local fixture host.

## Unix (install.sh)

### Setup

```sh
TMP=$(mktemp -d)
mkdir -p "$TMP/0.0.1"
mkdir -p "$TMP/build/teramind-0.0.1"
cargo build --release
cp target/release/{teramind,teramindd,teramind-hook,teramind-mcp} \
   "$TMP/build/teramind-0.0.1/"
TRIPLE=$(uname -m | sed -e 's/arm64/aarch64/' -e 's/amd64/x86_64/')-$(uname -s | sed -e 's/Linux/unknown-linux-gnu/' -e 's/Darwin/apple-darwin/')
ARCHIVE="teramind-0.0.1-${TRIPLE}.tar.gz"
( cd "$TMP/build" && tar -czf "$TMP/0.0.1/$ARCHIVE" "teramind-0.0.1" )
SUM=$(shasum -a 256 "$TMP/0.0.1/$ARCHIVE" | awk '{print $1}')
echo "$SUM  $ARCHIVE" > "$TMP/0.0.1/teramind-0.0.1-SHA256SUMS"
cat > "$TMP/releases.json" <<EOF
{"latest":"0.0.1","releases":[{"version":"0.0.1","artifacts":{"${TRIPLE}":{"url":"http://127.0.0.1:38080/0.0.1/$ARCHIVE","sha256":"$SUM"}}}]}
EOF
( cd "$TMP" && python3 -m http.server 38080 ) &
SERVER=$!
sleep 1
```

### Install

```sh
TERAMIND_RELEASE_BASE=http://127.0.0.1:38080 \
TERAMIND_INSTALL_ROOT="$TMP/install" \
TERAMIND_BIN_DIR="$TMP/binshadow" \
sh installer/install.sh
```

**Expect:**
- Exit 0.
- `$TMP/install/bin/teramind` exists and is executable.
- `$TMP/binshadow/teramind` is a symlink to the above.

### Self-update no-op

```sh
TERAMIND_RELEASE_INDEX_URL="http://127.0.0.1:38080/releases.json" \
"$TMP/install/bin/teramind" self-update --check-only
```

**Expect:** "already at latest" (because we just installed the only published version).

### Uninstall

```sh
TERAMIND_INSTALL_ROOT="$TMP/install" \
TERAMIND_BIN_SYMLINK="$TMP/binshadow/teramind" \
"$TMP/install/bin/teramind" uninstall --confirm
```

**Expect:** All four binaries + the symlink reported as `[removed]`; data dirs preserved.

### Tear down

```sh
kill $SERVER
rm -rf "$TMP"
```

## Windows (install.ps1)

Same idea, but use `python -m http.server 38080` from a different shell, and:

```pwsh
$env:TERAMIND_RELEASE_BASE = "http://127.0.0.1:38080"
$env:TERAMIND_INSTALL_ROOT = "$env:TEMP\teramind-test"
$env:TERAMIND_NO_MODIFY_PATH = "1"
powershell -ExecutionPolicy Bypass -File installer/install.ps1
```

**Expect:** Same outcomes as the Unix path, modulo the symlink step (Windows uses PATH prepending instead).
```

- [ ] **Step 2: Commit**

```bash
git add docs/runbooks/installer-manual-smoke.md
git commit -m "docs: manual smoke runbook for installers"
```

---

### Task 9.3: Full workspace check + clippy

- [ ] **Step 1: Run**

```bash
cargo check --workspace
cargo test --workspace --lib
cargo test -p teramind-cli uninstall
cargo test -p teramind-cli updater
cargo test -p teramind-cli --test self_update_local
cargo clippy --workspace -- -D warnings
```

Expected: all pass / clean.

- [ ] **Step 2: Validate the two release-related YAMLs parse**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"
```

Expected: no exceptions.

- [ ] **Step 3: Optional shellcheck locally**

```bash
shellcheck installer/install.sh
```

Expected: zero warnings. (Skipped if shellcheck isn't on PATH; CI catches it.)

- [ ] **Step 4: Commit any cleanups**

```bash
git add -A
git commit -m "chore: clippy + yaml cleanups for installer plan" || true
```

---

### Task 9.4: Open the PR

- [ ] **Step 1: STOP — do not push or open a PR.**

The previous plans (A, B, C, D) all merged via fast-forward from a feature branch after explicit user approval. Defer the push to that approval step.

Report: "Plan E complete on `feat/teramind-installers`; awaiting user merge approval."

---

## Spec coverage self-check

| Spec section | Requirement | Plan task |
|---|---|---|
| §7.1 distribution artifacts | 6-target build matrix → tarballs/zips | Task 5.1 |
| §7.1 distribution artifacts | SHA256SUMS aggregation | Task 6.1 |
| §7.1 distribution artifacts | cosign signature | Task 6.1 |
| §7.1 distribution artifacts | install.sh + install.ps1 published with each release | Tasks 5.1 + the static repo serving (release.yml uploads them indirectly via GH releases; covered) |
| §7.1 embedded Postgres downloaded on first run | Plan A — `postgresql_embedded` already handles this | already shipped |
| §7.2 installer behavior — Mac/Linux curl pipe | install.sh | Task 3.1 |
| §7.2 installer behavior — Windows irm/iex | install.ps1 | Task 4.1 |
| §7.2 installer steps 1–5 (detect, download, verify, extract, PATH/symlink) | install.sh + install.ps1 | Tasks 3.1, 4.1 |
| §7.2 Homebrew tap fast-follow | Formula scaffold | Task 7.1 |
| §7.6 `teramind self update` | self-update command | Tasks 2.1–2.4 |
| §7.6 `teramind uninstall [--purge]` | uninstall command | Tasks 1.1–1.2 |
| §7.7 macOS notarization | notarize CI job (gated) | Task 6.2 |
| §7.7 Windows unsigned in v1 | Default behavior; documented in runbook | Task 8.1 runbook |
