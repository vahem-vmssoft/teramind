//! Corpus JSONL loader + DB ingest.

use serde::{Deserialize, Serialize};
use std::io::BufRead;
use std::path::Path;
use teramind_core::types::file_diff::Attribution;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRow {
    pub id: Uuid,
    pub agent_kind: String,
    pub cwd: String,
    pub project_tag: String,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRow {
    pub id: Uuid,
    pub session_id: Uuid,
    pub ordinal: i32,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    pub user_prompt: Option<String>,
    pub assistant_text: Option<String>,
    pub thinking: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRow {
    pub id: Uuid,
    pub turn_id: Uuid,
    pub ordinal: i32,
    pub name: String,
    pub input: serde_json::Value,
    pub output: String,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiffRow {
    pub id: Uuid,
    pub session_id: Uuid,
    pub turn_id: Option<Uuid>,
    pub file_path: String,
    pub rel_path: String,
    pub attribution: Attribution,
    pub language: Option<String>,
    pub pre_excerpt: String,
    pub post_excerpt: String,
    pub unified_diff: String,
    #[serde(with = "time::serde::rfc3339")]
    pub captured_at: OffsetDateTime,
}

#[derive(Debug, Default)]
pub struct Corpus {
    pub sessions: Vec<SessionRow>,
    pub turns: Vec<TurnRow>,
    pub tool_calls: Vec<ToolCallRow>,
    pub file_diffs: Vec<FileDiffRow>,
}

pub fn load(root: &Path) -> anyhow::Result<Corpus> {
    let dir = root.join("corpus");
    Ok(Corpus {
        sessions:   load_jsonl(&dir.join("sessions.jsonl"))?,
        turns:      load_jsonl(&dir.join("turns.jsonl"))?,
        tool_calls: load_jsonl(&dir.join("tool_calls.jsonl"))?,
        file_diffs: load_jsonl(&dir.join("file_diffs.jsonl"))?,
    })
}

fn load_jsonl<T: serde::de::DeserializeOwned>(path: &Path) -> anyhow::Result<Vec<T>> {
    if !path.exists() { return Ok(Vec::new()); }
    let f = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(f);
    let mut out = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() { continue; }
        out.push(serde_json::from_str::<T>(&line)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_session_row() -> SessionRow {
        SessionRow {
            id: Uuid::nil(),
            agent_kind: "claude_code".into(),
            cwd: "/tmp/proj".into(),
            project_tag: "rust-web".into(),
            started_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        }
    }

    #[test]
    fn jsonl_roundtrips_session_rows() {
        let dir = TempDir::new().unwrap();
        let corpus_dir = dir.path().join("corpus");
        std::fs::create_dir_all(&corpus_dir).unwrap();
        let row = sample_session_row();
        let line = serde_json::to_string(&row).unwrap();
        std::fs::write(corpus_dir.join("sessions.jsonl"), line).unwrap();
        let c = load(dir.path()).unwrap();
        assert_eq!(c.sessions.len(), 1);
        assert_eq!(c.sessions[0].project_tag, "rust-web");
    }

    #[test]
    fn missing_files_yield_empty_corpus() {
        let dir = TempDir::new().unwrap();
        let c = load(dir.path()).unwrap();
        assert!(c.sessions.is_empty());
        assert!(c.turns.is_empty());
    }
}
