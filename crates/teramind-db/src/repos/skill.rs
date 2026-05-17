use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::SkillId;
use uuid::Uuid;

#[allow(clippy::type_complexity)]
type CodifiedRow = (Uuid, String, String, Vec<String>, Vec<Uuid>);

#[derive(Clone)]
pub struct SkillRepo {
    pool: DbPool,
}

impl SkillRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn upsert_authored(
        &self,
        name: &str,
        description: &str,
        body: &str,
    ) -> Result<SkillId> {
        let r: (uuid::Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO skills (name, description, body, source)
            VALUES ($1,$2,$3,'authored')
            ON CONFLICT (name) DO UPDATE SET description=EXCLUDED.description, body=EXCLUDED.body, updated_at=now()
            RETURNING id
            "#)
            .bind(name).bind(description).bind(body)
            .fetch_one(self.pool.pg()).await?;
        Ok(SkillId(r.0))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_codified(
        &self,
        name: &str,
        description: &str,
        body: &str,
        source_session_ids: &[uuid::Uuid],
        applies_to_cwds: &[String],
    ) -> Result<SkillId> {
        let r: (uuid::Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO skills (name, description, body, source, source_session_ids, applies_to_cwds)
            VALUES ($1,$2,$3,'codified',$4,$5)
            ON CONFLICT (name) DO UPDATE SET
                description=EXCLUDED.description,
                body=EXCLUDED.body,
                source_session_ids=EXCLUDED.source_session_ids,
                applies_to_cwds=EXCLUDED.applies_to_cwds,
                updated_at=now()
            RETURNING id
            "#)
            .bind(name).bind(description).bind(body)
            .bind(source_session_ids).bind(applies_to_cwds)
            .fetch_one(self.pool.pg()).await?;
        Ok(SkillId(r.0))
    }

    /// List all codified skills. The glob match against `cwd` is done in the caller.
    /// Returns `(id, name, description, applies_to_cwds, seeded_from_count)`.
    pub async fn list_codified_for_cwd(&self, cwd: &str, limit: i64) -> Result<Vec<(SkillId, String, String, Vec<String>, i32)>> {
        let rows: Vec<CodifiedRow> = sqlx::query_as(
            r#"SELECT id, name, description, applies_to_cwds, source_session_ids
               FROM skills WHERE source = 'codified'
               ORDER BY updated_at DESC
               LIMIT $1"#)
            .bind(limit).fetch_all(self.pool.pg()).await?;
        // Glob matching happens in the caller (services::codify::glob).
        let _ = cwd;
        Ok(rows.into_iter()
            .map(|r| (SkillId(r.0), r.1, r.2, r.3, r.4.len() as i32))
            .collect())
    }
}
