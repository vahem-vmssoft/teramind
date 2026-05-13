use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::{AgentId, ProjectId, SessionId};
use time::OffsetDateTime;

#[derive(Clone)]
pub struct SessionRepo { pool: DbPool }

pub struct NewSession<'a> {
    pub agent_id: AgentId,
    pub agent_session_id: Option<&'a str>,
    pub cwd: &'a str,
    pub project_id: Option<ProjectId>,
    pub parent_session_id: Option<SessionId>,
    pub git_head: Option<&'a str>,
    pub git_branch: Option<&'a str>,
    pub os: &'a str,
    pub hostname: &'a str,
    pub user_login: &'a str,
    pub started_at: OffsetDateTime,
}

impl SessionRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    pub async fn insert(&self, n: NewSession<'_>) -> Result<SessionId> {
        let r: (uuid::Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO sessions (agent_id, agent_session_id, cwd, project_id, parent_session_id,
                                  git_head, git_branch, os, hostname, user_login, started_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
            RETURNING id
            "#)
            .bind(n.agent_id.0)
            .bind(n.agent_session_id)
            .bind(n.cwd)
            .bind(n.project_id.map(|p| p.0))
            .bind(n.parent_session_id.map(|p| p.0))
            .bind(n.git_head).bind(n.git_branch)
            .bind(n.os).bind(n.hostname).bind(n.user_login)
            .bind(n.started_at)
            .fetch_one(self.pool.pg()).await?;
        Ok(SessionId(r.0))
    }

    /// Insert a session with a caller-provided id (used by the daemon ingest path
    /// where the client generates the session id). Idempotent via DO NOTHING on
    /// conflict; returns the requested id.
    pub async fn insert_with_id(&self, id: SessionId, n: NewSession<'_>) -> Result<SessionId> {
        sqlx::query(
            r#"
            INSERT INTO sessions (id, agent_id, agent_session_id, cwd, project_id, parent_session_id,
                                  git_head, git_branch, os, hostname, user_login, started_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
            ON CONFLICT (id) DO NOTHING
            "#)
            .bind(id.0)
            .bind(n.agent_id.0)
            .bind(n.agent_session_id)
            .bind(n.cwd)
            .bind(n.project_id.map(|p| p.0))
            .bind(n.parent_session_id.map(|p| p.0))
            .bind(n.git_head).bind(n.git_branch)
            .bind(n.os).bind(n.hostname).bind(n.user_login)
            .bind(n.started_at)
            .execute(self.pool.pg()).await?;
        Ok(id)
    }

    pub async fn end(&self, id: SessionId, ended_at: OffsetDateTime, reason: &str) -> Result<()> {
        sqlx::query("UPDATE sessions SET ended_at=$1, end_reason=$2 WHERE id=$3 AND ended_at IS NULL")
            .bind(ended_at).bind(reason).bind(id.0)
            .execute(self.pool.pg()).await?;
        Ok(())
    }

    pub async fn append_metadata(&self, id: SessionId, key: &str, value: serde_json::Value) -> Result<()> {
        sqlx::query("UPDATE sessions SET metadata = metadata || jsonb_build_object($1, $2) WHERE id=$3")
            .bind(key).bind(value).bind(id.0)
            .execute(self.pool.pg()).await?;
        Ok(())
    }
}
