//! Persisted forwarder offset.
//!
//! Stored at `<raw_dir>/.sync-offset.json`. The forwarder writes the highest
//! shipped (file, byte-offset) pair after each successful POST /v1/ingest.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncOffset {
    /// JSONL filename relative to raw_dir, e.g. "2026-05-17.jsonl".
    pub file: Option<String>,
    /// Byte offset within that file (next byte to read).
    pub byte_offset: u64,
}

impl SyncOffset {
    pub fn path(raw_dir: &Path) -> PathBuf {
        raw_dir.join(".sync-offset.json")
    }

    pub fn load(raw_dir: &Path) -> Result<Self> {
        let p = Self::path(raw_dir);
        if !p.exists() { return Ok(SyncOffset::default()); }
        let s = std::fs::read_to_string(&p)?;
        Ok(serde_json::from_str(&s)?)
    }

    pub fn save(&self, raw_dir: &Path) -> Result<()> {
        let p = Self::path(raw_dir);
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string(self)?)?;
        std::fs::rename(&tmp, &p)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_roundtrips_empty() {
        let dir = tempfile::tempdir().unwrap();
        let off = SyncOffset::load(dir.path()).unwrap();
        assert!(off.file.is_none() && off.byte_offset == 0);
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let off = SyncOffset { file: Some("2026-05-17.jsonl".into()), byte_offset: 4096 };
        off.save(dir.path()).unwrap();
        let loaded = SyncOffset::load(dir.path()).unwrap();
        assert_eq!(loaded.file.as_deref(), Some("2026-05-17.jsonl"));
        assert_eq!(loaded.byte_offset, 4096);
    }

    #[test]
    fn save_is_atomic_no_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let off = SyncOffset { file: Some("x".into()), byte_offset: 100 };
        off.save(dir.path()).unwrap();
        // The tmp file must not linger.
        assert!(!dir.path().join(".sync-offset.json.tmp").exists());
        assert!(dir.path().join(".sync-offset.json").exists());
    }
}
