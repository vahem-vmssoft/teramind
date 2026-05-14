//! Pure diff math: language detection, unified diff, hunk-bounded excerpts.

use std::path::Path;
use sha2::{Digest, Sha256};
use similar::TextDiff;

/// Produce a unified diff string in `git diff --no-index` style.
/// Header uses `a/<rel>` and `b/<rel>` to match standard parsers.
/// Returns the empty string when `pre == post`.
pub fn unified_diff(pre: &str, post: &str, rel_path: &str) -> String {
    if pre == post {
        return String::new();
    }
    let diff = TextDiff::from_lines(pre, post);
    let mut out = String::new();
    out.push_str(&format!("--- a/{rel_path}\n"));
    out.push_str(&format!("+++ b/{rel_path}\n"));
    for hunk in diff.unified_diff().context_radius(3).iter_hunks() {
        out.push_str(&hunk.to_string());
    }
    out
}

/// SHA-256 of arbitrary bytes, returned as a fixed-size 32-byte array
/// to match the `file_diffs.pre_hash` / `post_hash` columns.
pub fn sha256_hash(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    arr
}

/// Map a file extension to a coarse-grained language tag stored on `file_diffs.language`.
/// Returns `None` for unknown/binary/extensionless paths.
pub fn language_from_extension(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => "rust",
        "py" => "python",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "rb" => "ruby",
        "php" => "php",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hxx" => "cpp",
        "cs" => "csharp",
        "scala" => "scala",
        "sh" | "bash" | "zsh" => "shell",
        "sql" => "sql",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "json" => "json",
        "md" | "markdown" => "markdown",
        "html" | "htm" => "html",
        "css" | "scss" | "less" => "css",
        _ => return None,
    })
}

/// Extract ±`radius` line windows around each change in (pre, post).
///
/// Uses `similar::TextDiff` to identify changed line ranges, then projects
/// those ranges back onto the original line vectors with a `radius`-line
/// context. Overlapping windows merge.
///
/// Returns `(pre_excerpt, post_excerpt)`. Both empty when `pre == post`.
pub fn excerpts_around_hunks(pre: &str, post: &str, radius: usize) -> (String, String) {
    if pre == post {
        return (String::new(), String::new());
    }
    let pre_lines: Vec<&str> = pre.split_inclusive('\n').collect();
    let post_lines: Vec<&str> = post.split_inclusive('\n').collect();

    let mut pre_ranges: Vec<(usize, usize)> = Vec::new();
    let mut post_ranges: Vec<(usize, usize)> = Vec::new();
    let diff = TextDiff::from_lines(pre, post);
    for op in diff.ops() {
        // op gives (tag, old_start..old_end, new_start..new_end).
        let old = op.old_range();
        let new = op.new_range();
        // Only record changed segments (skip equal runs).
        if matches!(op.tag(), similar::DiffTag::Equal) {
            continue;
        }
        pre_ranges.push((old.start, old.end));
        post_ranges.push((new.start, new.end));
    }

    let pre_ex = collect_window(&pre_lines, &pre_ranges, radius);
    let post_ex = collect_window(&post_lines, &post_ranges, radius);
    (pre_ex, post_ex)
}

fn collect_window(lines: &[&str], ranges: &[(usize, usize)], radius: usize) -> String {
    if ranges.is_empty() {
        return String::new();
    }
    // Compute windows then merge overlaps.
    let mut windows: Vec<(usize, usize)> = ranges
        .iter()
        .map(|(s, e)| {
            let start = s.saturating_sub(radius);
            let end = (*e + radius).min(lines.len());
            (start, end)
        })
        .collect();
    windows.sort_by_key(|w| w.0);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for w in windows {
        match merged.last_mut() {
            Some(prev) if prev.1 >= w.0 => prev.1 = prev.1.max(w.1),
            _ => merged.push(w),
        }
    }
    let mut out = String::new();
    for (start, end) in merged {
        for line in &lines[start..end] {
            out.push_str(line);
        }
    }
    out
}

/// Plain-old-data payload assembled by `compute_file_diff` and consumed
/// by the FS watcher to build an `IngestEvent::FileDiff`.
#[derive(Debug, Clone)]
pub struct ComputedDiff {
    pub unified_diff: String,
    pub pre_excerpt: String,
    pub post_excerpt: String,
    pub pre_hash: [u8; 32],
    pub post_hash: [u8; 32],
    pub byte_size: i32,
    pub language: Option<String>,
}

/// Compute everything we need to persist for a (pre, post, path) triple.
/// Returns `None` when `pre == post`.
pub fn compute_file_diff(pre: &str, post: &str, rel_path: &Path) -> Option<ComputedDiff> {
    if pre == post {
        return None;
    }
    let rel = rel_path.to_string_lossy();
    let unified = unified_diff(pre, post, &rel);
    let (pre_ex, post_ex) = excerpts_around_hunks(pre, post, 50);
    Some(ComputedDiff {
        unified_diff: unified,
        pre_excerpt: pre_ex,
        post_excerpt: post_ex,
        pre_hash: sha256_hash(pre.as_bytes()),
        post_hash: sha256_hash(post.as_bytes()),
        byte_size: post.len() as i32,
        language: language_from_extension(rel_path).map(String::from),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn detects_common_languages() {
        assert_eq!(language_from_extension(&PathBuf::from("a.rs")), Some("rust"));
        assert_eq!(language_from_extension(&PathBuf::from("a.PY")), Some("python"));
        assert_eq!(language_from_extension(&PathBuf::from("a.tsx")), Some("typescript"));
    }

    #[test]
    fn unknown_extension_returns_none() {
        assert_eq!(language_from_extension(&PathBuf::from("a.xyz")), None);
        assert_eq!(language_from_extension(&PathBuf::from("Makefile")), None);
    }

    #[test]
    fn sha256_hash_is_stable() {
        let h1 = sha256_hash(b"hello");
        let h2 = sha256_hash(b"hello");
        assert_eq!(h1, h2);
        let h3 = sha256_hash(b"world");
        assert_ne!(h1, h3);
        // Known-answer test:
        let expected = hex::decode("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824").unwrap();
        assert_eq!(&h1[..], expected.as_slice());
    }

    #[test]
    fn unified_diff_emits_hunks_for_changed_text() {
        let pre = "line1\nline2\nline3\n";
        let post = "line1\nLINE TWO\nline3\n";
        let diff = unified_diff(pre, post, "foo.txt");
        assert!(diff.contains("--- a/foo.txt"), "diff: {diff}");
        assert!(diff.contains("+++ b/foo.txt"));
        assert!(diff.contains("-line2"));
        assert!(diff.contains("+LINE TWO"));
    }

    #[test]
    fn unified_diff_empty_when_identical() {
        let s = "same\n";
        assert!(unified_diff(s, s, "x").is_empty());
    }

    #[test]
    fn excerpts_extract_50_line_window_around_hunk() {
        // 200-line file; change happens at line 100.
        let pre_lines: Vec<String> = (1..=200).map(|i| format!("line{i}")).collect();
        let mut post_lines = pre_lines.clone();
        post_lines[99] = "CHANGED".to_string();
        let pre = pre_lines.join("\n") + "\n";
        let post = post_lines.join("\n") + "\n";

        let (pre_ex, post_ex) = excerpts_around_hunks(&pre, &post, 50);
        // Expect lines 50..=150 in the excerpt (100 ± 50).
        assert!(pre_ex.contains("line50\n"), "missing line50:\n{pre_ex}");
        assert!(pre_ex.contains("line100\n"));
        assert!(pre_ex.contains("line150\n"));
        assert!(!pre_ex.contains("line49\n"), "excerpt should not include line49");
        assert!(!pre_ex.contains("line151\n"));

        assert!(post_ex.contains("CHANGED\n"));
    }

    #[test]
    fn excerpts_handle_small_file() {
        let pre = "a\nb\nc\n";
        let post = "a\nB\nc\n";
        let (pre_ex, post_ex) = excerpts_around_hunks(pre, post, 50);
        assert!(pre_ex.contains("a\n") && pre_ex.contains("b\n") && pre_ex.contains("c\n"));
        assert!(post_ex.contains("B\n"));
    }

    #[test]
    fn excerpts_empty_when_unchanged() {
        let (pre_ex, post_ex) = excerpts_around_hunks("same\n", "same\n", 50);
        assert!(pre_ex.is_empty());
        assert!(post_ex.is_empty());
    }

    #[test]
    fn compute_file_diff_assembles_full_payload() {
        let pre = "fn old() {}\n";
        let post = "fn new() {}\n";
        let path = PathBuf::from("src/lib.rs");
        let d = compute_file_diff(pre, post, &path).expect("Some when changed");
        assert_eq!(d.language.as_deref(), Some("rust"));
        assert!(d.unified_diff.contains("-fn old() {}"));
        assert!(d.unified_diff.contains("+fn new() {}"));
        assert!(d.pre_excerpt.contains("fn old"));
        assert!(d.post_excerpt.contains("fn new"));
        assert_eq!(d.byte_size, post.len() as i32);
        assert_ne!(d.pre_hash, d.post_hash);
    }

    #[test]
    fn compute_file_diff_none_when_unchanged() {
        let s = "x";
        assert!(compute_file_diff(s, s, &PathBuf::from("a.rs")).is_none());
    }

    use proptest::prelude::*;

    proptest! {
        // No matter what (pre, post) we throw at it, the returned excerpts
        // must be subsets of pre/post respectively, and the diff parses as
        // a valid unified diff header when non-empty.
        #[test]
        fn excerpts_are_substrings_of_inputs(
            pre in proptest::collection::vec("[a-zA-Z0-9 ]{0,40}", 0..50),
            mutations in proptest::collection::vec(0u8..=3u8, 0..20),
        ) {
            let pre = pre.join("\n") + "\n";
            // Build a post by applying simple mutations.
            let mut post_lines: Vec<String> = pre.lines().map(|s| s.to_string()).collect();
            for (i, m) in mutations.iter().enumerate() {
                if post_lines.is_empty() { break; }
                let idx = i % post_lines.len();
                match m {
                    0 => post_lines[idx].push('!'),
                    1 => post_lines[idx].insert(0, '#'),
                    2 => { post_lines.remove(idx); }
                    _ => post_lines.insert(idx, "INS".into()),
                }
            }
            let post = post_lines.join("\n") + "\n";
            let (pre_ex, post_ex) = excerpts_around_hunks(&pre, &post, 5);
            // Every line in pre_ex must appear in pre; same for post_ex.
            for line in pre_ex.lines() {
                prop_assert!(pre.lines().any(|p| p == line),
                    "pre_excerpt line {:?} not in pre", line);
            }
            for line in post_ex.lines() {
                prop_assert!(post.lines().any(|p| p == line),
                    "post_excerpt line {:?} not in post", line);
            }
        }
    }
}
