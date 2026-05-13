use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::SkillId;

#[derive(Clone)]
pub struct SkillRepo { pool: DbPool }

impl SkillRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }
    pub async fn upsert_authored(&self, name: &str, description: &str, body: &str) -> Result<SkillId> {
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
}
