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
    parse_semver(c)
        .map(|cv| Some(cv) < parse_semver(l))
        .unwrap_or(false)
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
        artifacts.insert(
            "x86_64-unknown-linux-gnu".into(),
            Artifact {
                url: "u".into(),
                sha256: "s".into(),
            },
        );
        ReleaseIndex {
            latest: v.into(),
            releases: vec![Release {
                version: v.into(),
                artifacts,
            }],
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
