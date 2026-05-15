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

use teramind_core::ids::{FileDiffId, SessionId, ToolCallId, TurnId};
use teramind_db::pool::DbPool;
use teramind_db::repos::diff::NewFileDiff;
use teramind_db::repos::session::NewSession;
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo};

pub async fn ingest(pool: &DbPool, c: &Corpus) -> anyhow::Result<()> {
    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let diffs = DiffRepo::new(pool.clone());

    let claude_agent = agents.upsert("claude_code", None).await?;

    for s in &c.sessions {
        let _ = sessions.insert_with_id(SessionId(s.id), NewSession {
            agent_id: claude_agent.id,
            agent_session_id: None,
            cwd: &s.cwd,
            project_id: None,
            parent_session_id: None,
            git_head: None,
            git_branch: None,
            os: "linux",
            hostname: "eval",
            user_login: "eval",
            started_at: s.started_at,
        }).await?;
    }
    for t in &c.turns {
        let tid = trace.upsert_turn_with_id(
            TurnId(t.id),
            SessionId(t.session_id),
            t.ordinal,
            t.started_at,
            t.user_prompt.as_deref(),
        ).await?;
        trace.finalize_turn(
            tid, t.started_at,
            t.assistant_text.as_deref(),
            t.thinking.as_deref(),
            Some("eval-model"), None, None,
        ).await?;
    }
    for tc in &c.tool_calls {
        let _ = trace.insert_tool_call_start_with_id(
            ToolCallId(tc.id),
            TurnId(tc.turn_id),
            tc.ordinal, &tc.name, &tc.input, tc.started_at,
        ).await?;
        trace.finalize_tool_call(ToolCallId(tc.id), &tc.output, false, 0).await?;
    }
    for d in &c.file_diffs {
        let _: FileDiffId = diffs.insert(NewFileDiff {
            turn_id: d.turn_id.map(TurnId),
            session_id: SessionId(d.session_id),
            file_path: &d.file_path,
            rel_path: &d.rel_path,
            attribution: d.attribution,
            language: d.language.as_deref(),
            pre_excerpt: &d.pre_excerpt,
            post_excerpt: &d.post_excerpt,
            unified_diff: &d.unified_diff,
            pre_hash: [0u8; 32],
            post_hash: [1u8; 32],
            byte_size: d.post_excerpt.len() as i32,
            captured_at: d.captured_at,
        }).await?;
    }

    sqlx::query("REFRESH MATERIALIZED VIEW traces_fts")
        .execute(pool.pg()).await?;

    Ok(())
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

    use teramind_db::{migrate, pg_supervisor::PgSupervisor};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ingest_loads_a_minimal_corpus_into_pg() -> anyhow::Result<()> {
        let dir = TempDir::new().unwrap();
        let pgdata = dir.path().join("pgdata");
        let sup = PgSupervisor::start(pgdata, "teramind").await?;
        let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
        migrate::run(&pool).await?;

        let c = Corpus {
            sessions: vec![SessionRow {
                id: Uuid::new_v4(),
                agent_kind: "claude_code".into(),
                cwd: "/proj".into(),
                project_tag: "rust-web".into(),
                started_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            }],
            turns: vec![], tool_calls: vec![], file_diffs: vec![],
        };
        ingest(&pool, &c).await?;

        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM sessions")
            .fetch_one(pool.pg()).await?;
        assert_eq!(n, 1);

        sup.shutdown().await?;
        Ok(())
    }
}
