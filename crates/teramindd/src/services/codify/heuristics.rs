//! Shared heuristics for the codifier's pattern detectors.

use once_cell::sync::Lazy;
use regex::Regex;

/// Regex set indicating a turn is error-shaped (caller decides what counts).
static ERROR_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    [
        r"(?m)^error:",
        r"(?m)^Error:",
        r"panicked at",
        r"^Traceback",
        r"FAILED",
        r"clippy::[a-z_]+",
        r"cannot find ",
        r"undefined reference",
    ].iter().map(|p| Regex::new(p).unwrap()).collect()
});

pub fn looks_like_error(text: &str) -> bool {
    ERROR_PATTERNS.iter().any(|r| r.is_match(text))
}

/// Normalize an error string for signature hashing:
/// - Strip line/column numbers (`:123:45`).
/// - Replace generic identifiers `\w+` with `<id>` (but keep keywords like `error`, `Traceback`).
/// - Truncate to 80 chars.
pub fn normalize_error(text: &str) -> String {
    let re_line = Regex::new(r":\d+(:\d+)?").unwrap();
    let re_ident = Regex::new(r"\b[a-zA-Z_][a-zA-Z0-9_]+\b").unwrap();
    let no_lines = re_line.replace_all(text, "");
    let keywords = ["error", "Error", "panicked", "Traceback", "FAILED", "cannot", "find", "undefined", "reference", "clippy"];
    let normalized = re_ident.replace_all(&no_lines, |caps: &regex::Captures| {
        let word = &caps[0];
        if keywords.iter().any(|k| word.eq_ignore_ascii_case(k)) {
            word.to_string()
        } else {
            "<id>".to_string()
        }
    });
    normalized.chars().take(80).collect()
}

/// Classify a unified diff string into a coarse kind. Heuristic, not AST-aware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    AddedBlock,
    RemovedBlock,
    SignatureChange,
    Rename,
    Mixed,
}

impl DiffKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DiffKind::AddedBlock     => "added_block",
            DiffKind::RemovedBlock   => "removed_block",
            DiffKind::SignatureChange=> "signature_change",
            DiffKind::Rename         => "rename",
            DiffKind::Mixed          => "mixed",
        }
    }
}

pub fn classify_diff(diff: &str) -> DiffKind {
    let adds = diff.lines().filter(|l| l.starts_with('+') && !l.starts_with("+++")).count();
    let dels = diff.lines().filter(|l| l.starts_with('-') && !l.starts_with("---")).count();
    let renames = diff.lines().any(|l| l.starts_with("rename from ") || l.starts_with("similarity index "));
    let sig_change = diff.lines().any(|l| (l.starts_with('+') || l.starts_with('-')) &&
        (l.contains("fn ") || l.contains("def ") || l.contains("function ")));

    if renames { return DiffKind::Rename; }
    if sig_change { return DiffKind::SignatureChange; }
    if adds > 0 && dels == 0 { return DiffKind::AddedBlock; }
    if dels > 0 && adds == 0 { return DiffKind::RemovedBlock; }
    DiffKind::Mixed
}

/// Head verb of a Bash command: first whitespace-separated token, lowercased.
pub fn bash_head_verb(cmd: &str) -> &str {
    cmd.split_whitespace().next().unwrap_or("").trim_start_matches("./")
}

/// Extension of a file path, or `_` if none.
pub fn file_kind(path: &str) -> String {
    match path.rsplit('.').next() {
        Some(ext) if ext != path && !ext.is_empty() => format!(".{ext}"),
        _ => "_".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_patterns_detect_common_shapes() {
        assert!(looks_like_error("error: expected `;`"));
        assert!(looks_like_error("thread 'main' panicked at foo.rs"));
        assert!(looks_like_error("Traceback (most recent call last):"));
        assert!(looks_like_error("FAILED: 3 tests"));
        assert!(!looks_like_error("everything is fine"));
    }

    #[test]
    fn normalize_strips_line_numbers_and_identifiers() {
        let a = normalize_error("error: cannot find `foo` at file.rs:42:10");
        let b = normalize_error("error: cannot find `bar` at other.rs:99:1");
        assert_eq!(a, b, "line numbers and ident names must collapse to the same form");
    }

    #[test]
    fn classify_diff_kinds() {
        assert_eq!(classify_diff("+ added line\n"), DiffKind::AddedBlock);
        assert_eq!(classify_diff("- removed line\n"), DiffKind::RemovedBlock);
        assert_eq!(classify_diff("- pub fn foo() {}\n+ pub fn foo(x: i32) {}\n"), DiffKind::SignatureChange);
        assert_eq!(classify_diff("rename from a.rs\n"), DiffKind::Rename);
        assert_eq!(classify_diff("+ a\n- b\n"), DiffKind::Mixed);
    }

    #[test]
    fn bash_head_verb_basic() {
        assert_eq!(bash_head_verb("cargo build --release"), "cargo");
        assert_eq!(bash_head_verb("./scripts/run.sh foo"), "scripts/run.sh");
    }

    #[test]
    fn file_kind_returns_dot_ext_or_underscore() {
        assert_eq!(file_kind("foo.rs"), ".rs");
        assert_eq!(file_kind("path/to/Cargo.toml"), ".toml");
        assert_eq!(file_kind("Makefile"), "_");
    }
}
