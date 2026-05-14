//! Path filter for the FS watcher. Combines a built-in deny list with the
//! project's `.gitignore`.

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::{Path, PathBuf};

/// Built-in patterns that always match, in addition to .gitignore.
const ALWAYS_IGNORE: &[&str] = &[
    ".git/",
    ".git/**",
    "node_modules/",
    "node_modules/**",
    "target/",
    "target/**",
    "dist/",
    "dist/**",
    ".DS_Store",
    "*.swp",
    "*.swo",
    "*.tmp",
    "*~",
    "*.orig",
    ".idea/",
    ".idea/**",
    ".vscode/",
    ".vscode/**",
];

use std::sync::Arc;

/// `ignore::gitignore::Gitignore` is not `Clone`, so we wrap each instance
/// in `Arc` to keep `IgnoreFilter` cheaply cloneable for use inside the
/// `notify` event closure.
#[derive(Clone)]
pub struct IgnoreFilter {
    root: PathBuf,
    always: Arc<Gitignore>,
    project: Option<Arc<Gitignore>>,
}

impl IgnoreFilter {
    /// Build a filter rooted at `root`. Reads `<root>/.gitignore` when present.
    pub fn for_root(root: &Path) -> Self {
        let mut b = GitignoreBuilder::new(root);
        for p in ALWAYS_IGNORE {
            // unwrap: only fails if pattern is invalid, and ours are static.
            b.add_line(None, p).expect("static pattern");
        }
        let always = Arc::new(b.build().expect("build always-ignore"));

        let project = {
            let gi_path = root.join(".gitignore");
            if gi_path.exists() {
                let mut pb = GitignoreBuilder::new(root);
                let _ = pb.add(&gi_path);
                pb.build().ok().map(Arc::new)
            } else {
                None
            }
        };

        Self {
            root: root.to_path_buf(),
            always,
            project,
        }
    }

    /// Returns true when `abs_path` should be ignored.
    pub fn is_ignored(&self, abs_path: &Path) -> bool {
        // Check if any ancestor is ignored (for directory matching in .gitignore)
        let mut current = Some(abs_path);
        while let Some(path) = current {
            if path == self.root {
                break;
            }
            let is_dir = path.is_dir() || path.is_symlink();
            if self.always.matched(path, is_dir).is_ignore() {
                return true;
            }
            if let Some(g) = &self.project {
                if g.matched(path, is_dir).is_ignore() {
                    return true;
                }
            }
            current = path.parent();
        }
        false
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_tree() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        std::fs::write(dir.path().join("target/debug/x"), "x").unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn a(){}").unwrap();
        std::fs::write(dir.path().join(".DS_Store"), "x").unwrap();
        std::fs::write(dir.path().join("a.rs.swp"), "x").unwrap();
        dir
    }

    #[test]
    fn ignores_git_and_target_and_editor_junk() {
        let dir = make_tree();
        let f = IgnoreFilter::for_root(dir.path());
        assert!(f.is_ignored(&dir.path().join(".git/HEAD")));
        assert!(f.is_ignored(&dir.path().join("target/debug/x")));
        assert!(f.is_ignored(&dir.path().join(".DS_Store")));
        assert!(f.is_ignored(&dir.path().join("a.rs.swp")));
        assert!(!f.is_ignored(&dir.path().join("a.rs")));
    }

    #[test]
    fn respects_project_gitignore() {
        let dir = make_tree();
        std::fs::write(dir.path().join(".gitignore"), "secret.txt\nbuild/\n").unwrap();
        std::fs::write(dir.path().join("secret.txt"), "x").unwrap();
        std::fs::create_dir_all(dir.path().join("build")).unwrap();
        std::fs::write(dir.path().join("build/out"), "x").unwrap();

        let f = IgnoreFilter::for_root(dir.path());
        assert!(f.is_ignored(&dir.path().join("secret.txt")));
        assert!(f.is_ignored(&dir.path().join("build")));
        assert!(f.is_ignored(&dir.path().join("build/out")));
        assert!(!f.is_ignored(&dir.path().join("a.rs")));
    }
}
