use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::UserId;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone)]
pub struct UserRepo {
    pool: DbPool,
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: UserId,
    pub email: String,
    pub display_name: Option<String>,
    pub created_at: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
}

impl UserRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    pub async fn upsert_by_email(&self, email: &str, display_name: Option<&str>) -> Result<User> {
        let row: (Uuid, String, Option<String>, OffsetDateTime, Option<OffsetDateTime>) =
            sqlx::query_as(
                r#"
                INSERT INTO users (email, display_name)
                VALUES ($1, $2)
                ON CONFLICT (email) DO UPDATE SET display_name = COALESCE(EXCLUDED.display_name, users.display_name)
                RETURNING id, email, display_name, created_at, revoked_at
                "#)
            .bind(email).bind(display_name)
            .fetch_one(self.pool.pg()).await?;
        Ok(User { id: UserId(row.0), email: row.1, display_name: row.2,
                  created_at: row.3, revoked_at: row.4 })
    }

    pub async fn get_by_id(&self, id: UserId) -> Result<Option<User>> {
        let row: Option<(Uuid, String, Option<String>, OffsetDateTime, Option<OffsetDateTime>)> =
            sqlx::query_as(
                "SELECT id, email, display_name, created_at, revoked_at FROM users WHERE id = $1")
            .bind(id.0).fetch_optional(self.pool.pg()).await?;
        Ok(row.map(|r| User { id: UserId(r.0), email: r.1, display_name: r.2,
                              created_at: r.3, revoked_at: r.4 }))
    }

    pub async fn get_active(&self, id: UserId) -> Result<Option<User>> {
        Ok(self.get_by_id(id).await?.filter(|u| u.revoked_at.is_none()))
    }

    pub async fn revoke(&self, id: UserId) -> Result<()> {
        sqlx::query("UPDATE users SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL")
            .bind(id.0).execute(self.pool.pg()).await?;
        Ok(())
    }

    pub async fn list_all(&self) -> Result<Vec<User>> {
        let rows: Vec<(Uuid, String, Option<String>, OffsetDateTime, Option<OffsetDateTime>)> =
            sqlx::query_as(
                "SELECT id, email, display_name, created_at, revoked_at FROM users ORDER BY email")
            .fetch_all(self.pool.pg()).await?;
        Ok(rows.into_iter()
            .map(|r| User { id: UserId(r.0), email: r.1, display_name: r.2,
                            created_at: r.3, revoked_at: r.4 })
            .collect())
    }
}
