//! Pure diff math: language detection, unified diff, hunk-bounded excerpts.

use std::path::Path;
use sha2::{Digest, Sha256};

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
}
