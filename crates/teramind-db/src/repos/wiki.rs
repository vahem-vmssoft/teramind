//! Storage layer for `wiki_pages` + `sessions_to_summarize` reads.

use crate::error::Result;
use crate::pool::DbPool;
use serde::{Deserialize, Serialize};
use teramind_core::ids::{SessionId, WikiPageId};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone)]
pub struct WikiRepo {
    pool: DbPool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiPage {
    pub id: WikiPageId,
    pub session_id: SessionId,
    pub model: String,
    pub content: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub generated_at: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct SessionToSummarize {
    pub session_id: SessionId,
    pub cwd: String,
    pub started_at: OffsetDateTime,
    pub ended_at: OffsetDateTime,
    pub end_reason: String,
}

impl WikiRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    /// Sessions that are ended AND lack a wiki_page for `model`.
    pub async fn fetch_sessions_to_summarize(&self, model: &str, limit: u32) -> Result<Vec<SessionToSummarize>> {
        let rows: Vec<(Uuid, String, OffsetDateTime, OffsetDateTime, Option<String>)> = sqlx::query_as(
            r#"
            SELECT v.session_id, v.cwd, v.started_at, v.ended_at, v.end_reason
            FROM   sessions_to_summarize v
            WHERE  NOT EXISTS (
                SELECT 1 FROM wiki_pages w
                WHERE  w.session_id = v.session_id
                  AND  w.model      = $1
            )
            ORDER  BY v.ended_at ASC
            LIMIT  $2
            "#,
        )
        .bind(model)
        .bind(limit as i64)
        .fetch_all(self.pool.pg()).await?;
        Ok(rows.into_iter().map(|(sid, cwd, started_at, ended_at, end_reason)| {
            SessionToSummarize {
                session_id: SessionId(sid),
                cwd, started_at, ended_at,
                end_reason: end_reason.unwrap_or_default(),
            }
        }).collect())
    }

    pub async fn upsert(
        &self,
        session_id: SessionId,
        model: &str,
        content: &str,
        input_tokens: u32,
        output_tokens: u32,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO wiki_pages (session_id, model, content, input_tokens, output_tokens)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (session_id, model) DO UPDATE SET
                content       = EXCLUDED.content,
                input_tokens  = EXCLUDED.input_tokens,
                output_tokens = EXCLUDED.output_tokens,
                generated_at  = now()
            "#,
        )
        .bind(session_id.0)
        .bind(model)
        .bind(content)
        .bind(input_tokens as i32)
        .bind(output_tokens as i32)
        .execute(self.pool.pg()).await?;
        Ok(())
    }

    /// Sentinel "skipped" mark — empty content prevents re-evaluation.
    pub async fn mark_skipped(&self, session_id: SessionId, model: &str) -> Result<()> {
        self.upsert(session_id, model, "", 0, 0).await
    }

    pub async fn get_for_session(&self, session_id: SessionId, model: &str) -> Result<Option<WikiPage>> {
        let row: Option<(Uuid, Uuid, String, String, i32, i32, OffsetDateTime)> = sqlx::query_as(
            r#"
            SELECT id, session_id, model, content, input_tokens, output_tokens, generated_at
            FROM   wiki_pages
            WHERE  session_id = $1 AND model = $2
            "#,
        )
        .bind(session_id.0)
        .bind(model)
        .fetch_optional(self.pool.pg()).await?;
        Ok(row.map(|(id, sid, m, c, it, ot, ts)| WikiPage {
            id: WikiPageId(id),
            session_id: SessionId(sid),
            model: m, content: c,
            input_tokens: it as u32, output_tokens: ot as u32,
            generated_at: ts,
        }))
    }

    /// Most-recent non-empty wiki for any session whose cwd matches.
    /// Empty content (sentinel skip) is excluded.
    pub async fn latest_for_cwd(&self, cwd: &str) -> Result<Option<WikiPage>> {
        let row: Option<(Uuid, Uuid, String, String, i32, i32, OffsetDateTime)> = sqlx::query_as(
            r#"
            SELECT w.id, w.session_id, w.model, w.content, w.input_tokens, w.output_tokens, w.generated_at
            FROM   wiki_pages w
            JOIN   sessions   s ON s.id = w.session_id
            WHERE  s.cwd = $1
              AND  length(w.content) > 0
            ORDER  BY w.generated_at DESC
            LIMIT  1
            "#,
        )
        .bind(cwd)
        .fetch_optional(self.pool.pg()).await?;
        Ok(row.map(|(id, sid, m, c, it, ot, ts)| WikiPage {
            id: WikiPageId(id),
            session_id: SessionId(sid),
            model: m, content: c,
            input_tokens: it as u32, output_tokens: ot as u32,
            generated_at: ts,
        }))
    }

    /// Count of ended sessions that lack a wiki for `model`.
    pub async fn backlog(&self, model: &str) -> Result<i64> {
        let (n,): (i64,) = sqlx::query_as(
            r#"
            SELECT count(*) FROM sessions_to_summarize v
            WHERE NOT EXISTS (
                SELECT 1 FROM wiki_pages w
                WHERE w.session_id = v.session_id AND w.model = $1
            )
            "#,
        )
        .bind(model)
        .fetch_one(self.pool.pg()).await?;
        Ok(n)
    }

    pub async fn load_snapshot(&self, session_id: SessionId) -> Result<Option<teramind_core::summarize::SessionSnapshot>> {
        // Session metadata.
        #[allow(clippy::type_complexity)]
        let row: Option<(Uuid, String, OffsetDateTime, Option<OffsetDateTime>, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
            r#"
            SELECT id, cwd, started_at, ended_at, end_reason, git_branch, git_head
            FROM   sessions WHERE id = $1
            "#,
        )
        .bind(session_id.0)
        .fetch_optional(self.pool.pg()).await?;
        let Some((_sid, cwd, started_at, ended_at, end_reason, git_branch, git_head)) = row else {
            return Ok(None);
        };
        let Some(ended_at) = ended_at else { return Ok(None) }; // un-ended

        // Turns.
        #[allow(clippy::type_complexity)]
        let turn_rows: Vec<(Uuid, i32, Option<String>, Option<String>, Option<String>, OffsetDateTime)> = sqlx::query_as(
            r#"
            SELECT id, ordinal, user_prompt, assistant_text, thinking, started_at
            FROM   turns WHERE session_id = $1 ORDER BY ordinal
            "#,
        )
        .bind(session_id.0).fetch_all(self.pool.pg()).await?;
        let turns = turn_rows.into_iter().map(|(id, ord, up, at, th, sa)| {
            teramind_core::summarize::TurnRow {
                id: teramind_core::ids::TurnId(id),
                ordinal: ord,
                user_prompt: up, assistant_text: at, thinking: th,
                started_at: sa,
            }
        }).collect::<Vec<_>>();

        // Tool calls.
        let tc_rows: Vec<(Uuid, Uuid, String, serde_json::Value, Option<String>, bool)> = sqlx::query_as(
            r#"
            SELECT tc.id, tc.turn_id, tc.name, tc.input, tc.output, tc.is_error
            FROM   tool_calls tc
            JOIN   turns t ON t.id = tc.turn_id
            WHERE  t.session_id = $1
            ORDER  BY tc.turn_id, tc.ordinal
            "#,
        )
        .bind(session_id.0).fetch_all(self.pool.pg()).await?;
        let tool_calls = tc_rows.into_iter().map(|(id, tid, name, input, output, is_error)| {
            teramind_core::summarize::ToolCallRow {
                id: teramind_core::ids::ToolCallId(id),
                turn_id: teramind_core::ids::TurnId(tid),
                name, input, output: output.unwrap_or_default(), is_error,
            }
        }).collect::<Vec<_>>();

        // File diffs.
        #[allow(clippy::type_complexity)]
        let fd_rows: Vec<(Option<Uuid>, String, Option<String>, String, String, String, String)> = sqlx::query_as(
            r#"
            SELECT turn_id, rel_path, language, attribution, unified_diff, pre_excerpt, post_excerpt
            FROM   file_diffs
            WHERE  session_id = $1
            ORDER  BY captured_at
            "#,
        )
        .bind(session_id.0).fetch_all(self.pool.pg()).await?;
        let file_diffs = fd_rows.into_iter().map(|(tid, rel, lang, attr, diff, pre, post)| {
            let attribution = match attr.as_str() {
                "agent" => teramind_core::types::file_diff::Attribution::Agent,
                _       => teramind_core::types::file_diff::Attribution::Human,
            };
            teramind_core::summarize::FileDiffRow {
                turn_id: tid.map(teramind_core::ids::TurnId),
                rel_path: rel,
                language: lang,
                attribution,
                unified_diff: diff,
                pre_excerpt: pre,
                post_excerpt: post,
            }
        }).collect::<Vec<_>>();

        Ok(Some(teramind_core::summarize::SessionSnapshot {
            session_id,
            cwd,
            started_at,
            ended_at,
            end_reason: end_reason.unwrap_or_default(),
            git_branch,
            git_head,
            turns,
            tool_calls,
            file_diffs,
        }))
    }
}
