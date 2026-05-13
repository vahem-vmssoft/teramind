use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::AgentId;
use teramind_core::types::Agent;
use time::OffsetDateTime;

#[derive(Clone)]
pub struct AgentRepo { pool: DbPool }

impl AgentRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    pub async fn upsert(&self, kind: &str, version: Option<&str>) -> Result<Agent> {
        let row: (uuid::Uuid, String, Option<String>, OffsetDateTime) = sqlx::query_as(
            r#"
            INSERT INTO agents (kind, version) VALUES ($1, $2)
            ON CONFLICT (kind, version) DO UPDATE SET kind = EXCLUDED.kind
            RETURNING id, kind, version, installed_at
            "#)
            .bind(kind)
            .bind(version)
            .fetch_one(self.pool.pg()).await?;
        Ok(Agent { id: AgentId(row.0), kind: row.1, version: row.2, installed_at: row.3 })
    }

    pub async fn find(&self, kind: &str, version: Option<&str>) -> Result<Option<Agent>> {
        let r: Option<(uuid::Uuid, String, Option<String>, OffsetDateTime)> = sqlx::query_as(
            "SELECT id, kind, version, installed_at FROM agents WHERE kind = $1 AND version IS NOT DISTINCT FROM $2")
            .bind(kind).bind(version)
            .fetch_optional(self.pool.pg()).await?;
        Ok(r.map(|r| Agent { id: AgentId(r.0), kind: r.1, version: r.2, installed_at: r.3 }))
    }
}
