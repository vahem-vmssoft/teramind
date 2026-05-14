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
