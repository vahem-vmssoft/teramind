use crate::error::Result;
use crate::pool::DbPool;
use serde_json::Value;
use teramind_core::ids::{SessionId, SkillObservationId};
use time::OffsetDateTime;
use uuid::Uuid;

type ObservationRow = (
    Uuid,
    String,
    String,
    Vec<Uuid>,
    i32,
    Value,
    OffsetDateTime,
    OffsetDateTime,
    String,
);

fn row_to_observation(r: ObservationRow) -> Observation {
    Observation {
        id: SkillObservationId(r.0),
        kind: r.1,
        signature: r.2,
        session_ids: r.3,
        frequency: r.4,
        context_blob: r.5,
        first_seen_at: r.6,
        last_seen_at: r.7,
        status: r.8,
    }
}

#[derive(Debug, Clone)]
pub struct Observation {
    pub id: SkillObservationId,
    pub kind: String,
    pub signature: String,
    pub session_ids: Vec<Uuid>,
    pub frequency: i32,
    pub context_blob: Value,
    pub first_seen_at: OffsetDateTime,
    pub last_seen_at: OffsetDateTime,
    pub status: String,
}

#[derive(Clone)]
pub struct SkillObservationRepo {
    pool: DbPool,
}

impl SkillObservationRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// UPSERT keyed by (kind, signature). Appends new session_ids and bumps frequency.
    pub async fn upsert(
        &self,
        kind: &str,
        signature: &str,
        new_sessions: &[SessionId],
        context_blob: Value,
    ) -> Result<()> {
        let new_uuids: Vec<Uuid> = new_sessions.iter().map(|s| s.0).collect();
        sqlx::query(
            r#"
            INSERT INTO skill_observations (kind, signature, session_ids, frequency, context_blob)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (kind, signature) DO UPDATE
              SET session_ids = (
                    SELECT ARRAY(SELECT DISTINCT unnest(skill_observations.session_ids || EXCLUDED.session_ids))
                  ),
                  frequency = (
                    SELECT cardinality(ARRAY(SELECT DISTINCT unnest(skill_observations.session_ids || EXCLUDED.session_ids)))
                  ),
                  last_seen_at = now(),
                  context_blob = EXCLUDED.context_blob
            "#)
            .bind(kind).bind(signature).bind(&new_uuids).bind(new_uuids.len() as i32)
            .bind(context_blob)
            .execute(self.pool.pg()).await?;
        Ok(())
    }

    pub async fn find_by_sig(&self, kind: &str, signature: &str) -> Result<Option<Observation>> {
        let row: Option<ObservationRow> = sqlx::query_as(
            r#"SELECT id, kind, signature, session_ids, frequency, context_blob,
                       first_seen_at, last_seen_at, status
               FROM skill_observations WHERE kind = $1 AND signature = $2"#,
        )
        .bind(kind)
        .bind(signature)
        .fetch_optional(self.pool.pg())
        .await?;
        Ok(row.map(row_to_observation))
    }

    pub async fn list_open(&self, min_frequency: i32, limit: i64) -> Result<Vec<Observation>> {
        let rows: Vec<ObservationRow> = sqlx::query_as(
            r#"SELECT id, kind, signature, session_ids, frequency, context_blob,
                       first_seen_at, last_seen_at, status
               FROM skill_observations
               WHERE status = 'open' AND frequency >= $1
               ORDER BY last_seen_at ASC
               LIMIT $2"#,
        )
        .bind(min_frequency)
        .bind(limit)
        .fetch_all(self.pool.pg())
        .await?;
        Ok(rows.into_iter().map(row_to_observation).collect())
    }

    pub async fn list_recent(
        &self,
        kind: Option<&str>,
        status: Option<&str>,
        limit: i64,
    ) -> Result<Vec<Observation>> {
        let rows: Vec<ObservationRow> = sqlx::query_as(
            r#"SELECT id, kind, signature, session_ids, frequency, context_blob,
                       first_seen_at, last_seen_at, status
               FROM skill_observations
               WHERE ($1::text IS NULL OR kind = $1)
                 AND ($2::text IS NULL OR status = $2)
               ORDER BY last_seen_at DESC
               LIMIT $3"#,
        )
        .bind(kind)
        .bind(status)
        .bind(limit)
        .fetch_all(self.pool.pg())
        .await?;
        Ok(rows.into_iter().map(row_to_observation).collect())
    }

    pub async fn mark_synthesized(&self, id: SkillObservationId) -> Result<()> {
        sqlx::query("UPDATE skill_observations SET status='synthesized' WHERE id=$1")
            .bind(id.0)
            .execute(self.pool.pg())
            .await?;
        Ok(())
    }

    pub async fn mark_skipped(&self, id: SkillObservationId, reason: &str) -> Result<()> {
        sqlx::query(
            r#"UPDATE skill_observations
               SET status='skipped',
                   context_blob = jsonb_set(context_blob, '{skip_reason}', to_jsonb($2::text))
               WHERE id=$1"#,
        )
        .bind(id.0)
        .bind(reason)
        .execute(self.pool.pg())
        .await?;
        Ok(())
    }
}
