use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::{SessionId, SkillCandidateId, SkillObservationId};
use time::OffsetDateTime;
use uuid::Uuid;

#[allow(clippy::type_complexity)]
type CandidateRow = (
    Uuid, Uuid, String, String, String,
    Vec<String>, Vec<Uuid>, String, i32, i32,
    OffsetDateTime, String, Option<String>, Option<OffsetDateTime>,
);

fn row_to_candidate(r: CandidateRow) -> Candidate {
    Candidate {
        id: SkillCandidateId(r.0),
        observation_id: SkillObservationId(r.1),
        name: r.2, description: r.3, body: r.4,
        applies_to_cwds: r.5,
        source_session_ids: r.6,
        model: r.7,
        input_tokens: r.8, output_tokens: r.9,
        generated_at: r.10,
        status: r.11,
        reviewer: r.12,
        reviewed_at: r.13,
    }
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: SkillCandidateId,
    pub observation_id: SkillObservationId,
    pub name: String,
    pub description: String,
    pub body: String,
    pub applies_to_cwds: Vec<String>,
    pub source_session_ids: Vec<Uuid>,
    pub model: String,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub generated_at: OffsetDateTime,
    pub status: String,
    pub reviewer: Option<String>,
    pub reviewed_at: Option<OffsetDateTime>,
}

#[derive(Clone)]
pub struct SkillCandidateRepo { pool: DbPool }

impl SkillCandidateRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert(
        &self,
        observation_id: SkillObservationId,
        name: &str,
        description: &str,
        body: &str,
        applies_to_cwds: &[String],
        source_session_ids: &[SessionId],
        model: &str,
        input_tokens: i32,
        output_tokens: i32,
    ) -> Result<SkillCandidateId> {
        let sids: Vec<Uuid> = source_session_ids.iter().map(|s| s.0).collect();
        let row: (Uuid,) = sqlx::query_as(
            r#"INSERT INTO skill_candidates
               (observation_id, name, description, body, applies_to_cwds,
                source_session_ids, model, input_tokens, output_tokens)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
               RETURNING id"#)
            .bind(observation_id.0)
            .bind(name).bind(description).bind(body)
            .bind(applies_to_cwds).bind(&sids)
            .bind(model).bind(input_tokens).bind(output_tokens)
            .fetch_one(self.pool.pg()).await?;
        Ok(SkillCandidateId(row.0))
    }

    pub async fn list_pending(&self, limit: i64) -> Result<Vec<Candidate>> {
        let rows: Vec<CandidateRow> = sqlx::query_as(
            r#"SELECT id, observation_id, name, description, body, applies_to_cwds,
                       source_session_ids, model, input_tokens, output_tokens,
                       generated_at, status, reviewer, reviewed_at
               FROM skill_candidates WHERE status='pending'
               ORDER BY generated_at DESC
               LIMIT $1"#)
            .bind(limit).fetch_all(self.pool.pg()).await?;
        Ok(rows.into_iter().map(row_to_candidate).collect())
    }

    pub async fn list_approved(&self, limit: i64) -> Result<Vec<Candidate>> {
        let rows: Vec<CandidateRow> = sqlx::query_as(
            r#"SELECT id, observation_id, name, description, body, applies_to_cwds,
                       source_session_ids, model, input_tokens, output_tokens,
                       generated_at, status, reviewer, reviewed_at
               FROM skill_candidates WHERE status='approved'
               ORDER BY reviewed_at ASC
               LIMIT $1
               FOR UPDATE SKIP LOCKED"#)
            .bind(limit).fetch_all(self.pool.pg()).await?;
        Ok(rows.into_iter().map(row_to_candidate).collect())
    }

    pub async fn list_filter(&self, status: Option<&str>, limit: i64) -> Result<Vec<Candidate>> {
        let rows: Vec<CandidateRow> = sqlx::query_as(
            r#"SELECT id, observation_id, name, description, body, applies_to_cwds,
                       source_session_ids, model, input_tokens, output_tokens,
                       generated_at, status, reviewer, reviewed_at
               FROM skill_candidates
               WHERE ($1::text IS NULL OR status = $1)
               ORDER BY generated_at DESC
               LIMIT $2"#)
            .bind(status).bind(limit)
            .fetch_all(self.pool.pg()).await?;
        Ok(rows.into_iter().map(row_to_candidate).collect())
    }

    pub async fn mark_promoted(&self, id: SkillCandidateId) -> Result<()> {
        sqlx::query("UPDATE skill_candidates SET status='promoted' WHERE id=$1")
            .bind(id.0).execute(self.pool.pg()).await?;
        Ok(())
    }

    /// Returns the number of candidates that were marked `superseded`
    /// (candidates from the same observation that were still `pending`).
    pub async fn supersede_prior(&self, observation_id: SkillObservationId, exclude_id: SkillCandidateId) -> Result<u64> {
        let r = sqlx::query(
            r#"UPDATE skill_candidates
               SET status='superseded'
               WHERE observation_id = $1 AND id != $2 AND status='pending'"#)
            .bind(observation_id.0).bind(exclude_id.0)
            .execute(self.pool.pg()).await?;
        Ok(r.rows_affected())
    }
}
