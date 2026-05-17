//! Per-project team-share marker: `.teramind/team-share.toml`.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareMarker {
    pub share: bool,
    pub set_by: String,
    #[serde(with = "time::serde::rfc3339")]
    pub set_at: time::OffsetDateTime,
}

/// Walk from `cwd` upward to `$HOME` looking for `.teramind/team-share.toml`.
/// Returns the first hit, or None.
pub fn find_marker(cwd: &Path, home: &Path) -> Option<(PathBuf, ShareMarker)> {
    let mut dir = cwd.canonicalize().ok()?;
    let home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    loop {
        let candidate = dir.join(".teramind").join("team-share.toml");
        if candidate.exists() {
            if let Ok(raw) = std::fs::read_to_string(&candidate) {
                if let Ok(m) = toml::from_str::<ShareMarker>(&raw) {
                    return Some((candidate, m));
                }
            }
        }
        if dir == home { break; }
        let Some(parent) = dir.parent() else { break; };
        if parent == dir { break; }
        dir = parent.to_path_buf();
    }
    None
}

/// Write the marker at `<cwd>/.teramind/team-share.toml`. Creates the dir.
pub fn write_marker_at_cwd(cwd: &Path, marker: &ShareMarker) -> Result<PathBuf> {
    let dir = cwd.join(".teramind");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("team-share.toml");
    let raw = toml::to_string(marker)?;
    std::fs::write(&path, raw)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_marker_in_self() {
        let dir = tempfile::tempdir().unwrap();
        let marker = ShareMarker {
            share: true, set_by: "alice".into(),
            set_at: time::OffsetDateTime::now_utc(),
        };
        write_marker_at_cwd(dir.path(), &marker).unwrap();
        let (path, m) = find_marker(dir.path(), &PathBuf::from("/"))
            .expect("marker must be findable from self");
        assert!(path.ends_with("team-share.toml"));
        assert!(m.share);
    }

    #[test]
    fn find_marker_walks_up() {
        let root = tempfile::tempdir().unwrap();
        let child = root.path().join("a/b/c");
        std::fs::create_dir_all(&child).unwrap();
        let marker = ShareMarker {
            share: false, set_by: "alice".into(),
            set_at: time::OffsetDateTime::now_utc(),
        };
        write_marker_at_cwd(root.path(), &marker).unwrap();
        let (_, m) = find_marker(&child, &PathBuf::from("/"))
            .expect("marker must be findable from a descendant");
        assert!(!m.share);
    }

    #[test]
    fn no_marker_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let result = find_marker(dir.path(), &PathBuf::from("/"));
        assert!(result.is_none());
    }

    #[test]
    fn walk_stops_at_home() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let proj = home.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        // Marker is *above* HOME — must not be found.
        let marker = ShareMarker {
            share: true, set_by: "x".into(),
            set_at: time::OffsetDateTime::now_utc(),
        };
        write_marker_at_cwd(root.path(), &marker).unwrap();
        assert!(find_marker(&proj, &home).is_none());
    }
}
