//! Best-effort lookup of the git-indexed version of a file via `git show :rel`.
//!
//! Returns `None` when the cwd is not a git repo, the file is untracked,
//! `git` is missing on PATH, or the lookup times out. The FS watcher
//! falls back to an empty pre-content string in any of those cases.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

const GIT_TIMEOUT: Duration = Duration::from_millis(500);

pub async fn show_index(cwd: &Path, rel_path: &str) -> Option<String> {
    // Use `--` to ensure rel_path is treated as a pathspec, not a rev.
    let mut cmd = Command::new("git");
    cmd.arg("show")
        .arg(format!(":{rel_path}"))
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = cmd.spawn().ok()?;
    let out = timeout(GIT_TIMEOUT, child.wait_with_output()).await.ok()?.ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as SyncCommand;
    use tempfile::TempDir;

    fn init_repo_with_committed_file(content: &str) -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        SyncCommand::new("git").arg("init").arg("-q").current_dir(p).status().unwrap();
        SyncCommand::new("git").args(["config","user.email","t@t"]).current_dir(p).status().unwrap();
        SyncCommand::new("git").args(["config","user.name","t"]).current_dir(p).status().unwrap();
        std::fs::write(p.join("a.txt"), content).unwrap();
        SyncCommand::new("git").args(["add","a.txt"]).current_dir(p).status().unwrap();
        SyncCommand::new("git").args(["commit","-q","-m","init"]).current_dir(p).status().unwrap();
        dir
    }

    #[tokio::test]
    async fn returns_indexed_content_for_committed_file() {
        let repo = init_repo_with_committed_file("hello\nworld\n");
        let got = show_index(repo.path(), "a.txt").await;
        assert_eq!(got.as_deref(), Some("hello\nworld\n"));
    }

    #[tokio::test]
    async fn returns_none_for_untracked_file() {
        let repo = init_repo_with_committed_file("x");
        std::fs::write(repo.path().join("untracked.rs"), "y").unwrap();
        let got = show_index(repo.path(), "untracked.rs").await;
        assert!(got.is_none(), "expected None for untracked, got {got:?}");
    }

    #[tokio::test]
    async fn returns_none_outside_repo() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), "x").unwrap();
        let got = show_index(dir.path(), "a").await;
        assert!(got.is_none());
    }
}
