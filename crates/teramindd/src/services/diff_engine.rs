//! Pure diff math: language detection, unified diff, hunk-bounded excerpts.

use std::path::Path;

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
}
