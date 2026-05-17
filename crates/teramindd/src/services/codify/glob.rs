//! Minimal cwd glob matcher. Supports:
//! - Plain prefix:        `/Users/alice/proj`   matches `/Users/alice/proj/sub/file`
//! - `*` segment wildcard: `/openvms-*`        matches `/openvms-rsync`, `/openvms-llvm`
//! - Empty pattern list = global (matches all).
//!
//! We do NOT use a full glob crate — the language is intentionally tiny so
//! the SessionStart digest filter is O(N skills × M patterns) with cheap
//! per-comparison work.

pub fn matches(pattern: &str, cwd: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }
    // Plain prefix (no `*`): treat as ancestor match.
    if !pattern.contains('*') {
        return cwd == pattern || cwd.starts_with(&format!("{pattern}/"));
    }
    // Segment-wildcard match.
    let pat_segs: Vec<&str> = pattern.trim_start_matches('/').split('/').collect();
    let cwd_segs: Vec<&str> = cwd.trim_start_matches('/').split('/').collect();
    if pat_segs.len() > cwd_segs.len() {
        return false;
    }
    for (p, c) in pat_segs.iter().zip(cwd_segs.iter()) {
        if !segment_matches(p, c) {
            return false;
        }
    }
    true
}

pub fn matches_any(patterns: &[String], cwd: &str) -> bool {
    if patterns.is_empty() {
        return true;
    } // global
    patterns.iter().any(|p| matches(p, cwd))
}

fn segment_matches(pat: &str, seg: &str) -> bool {
    if pat == "*" {
        return true;
    }
    if !pat.contains('*') {
        return pat == seg;
    }
    // Simple two-side wildcard: `prefix*suffix`. Only one `*` supported.
    let parts: Vec<&str> = pat.splitn(2, '*').collect();
    let (pre, post) = (parts[0], parts[1]);
    seg.starts_with(pre) && seg.ends_with(post) && seg.len() >= pre.len() + post.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_prefix_matches_self_and_descendants() {
        assert!(matches("/Users/alice/proj", "/Users/alice/proj"));
        assert!(matches("/Users/alice/proj", "/Users/alice/proj/sub"));
        assert!(!matches("/Users/alice/proj", "/Users/alice/other"));
        assert!(!matches("/Users/alice/proj", "/Users/alice/projection"));
    }

    #[test]
    fn segment_wildcard_matches() {
        assert!(matches("/openvms-*", "/openvms-rsync"));
        assert!(matches("/openvms-*", "/openvms-llvm"));
        assert!(matches("/openvms-*", "/openvms-rsync/src"));
        assert!(!matches("/openvms-*", "/openssl-vms"));
    }

    #[test]
    fn empty_pattern_does_not_match() {
        assert!(!matches("", "/anything"));
    }

    #[test]
    fn matches_any_empty_is_global() {
        assert!(matches_any(&[], "/anywhere"));
    }

    #[test]
    fn matches_any_with_patterns() {
        let ps = vec!["/openvms-*".to_string(), "/Users/alice/proj".to_string()];
        assert!(matches_any(&ps, "/openvms-rsync"));
        assert!(matches_any(&ps, "/Users/alice/proj/sub"));
        assert!(!matches_any(&ps, "/some/other/path"));
    }
}
