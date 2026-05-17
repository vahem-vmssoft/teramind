use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::{DeviceId, UserId};
use time::OffsetDateTime;
use uuid::Uuid;

type DeviceRow = (Uuid, Uuid, String, Vec<u8>, Option<OffsetDateTime>);

fn row_to_device(r: DeviceRow) -> Device {
    Device {
        id: DeviceId(r.0),
        user_id: UserId(r.1),
        name: r.2,
        public_key: r.3,
        last_seen_at: r.4,
    }
}

#[derive(Clone)]
pub struct DeviceRepo {
    pool: DbPool,
}

#[derive(Debug, Clone)]
pub struct Device {
    pub id: DeviceId,
    pub user_id: UserId,
    pub name: String,
    pub public_key: Vec<u8>,
    pub last_seen_at: Option<OffsetDateTime>,
}

impl DeviceRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn insert(
        &self,
        user_id: UserId,
        name: &str,
        token_hash: &[u8],
        public_key: &[u8],
    ) -> Result<Device> {
        let row: DeviceRow = sqlx::query_as(
            r#"
            INSERT INTO devices (user_id, name, token_hash, public_key)
            VALUES ($1, $2, $3, $4)
            RETURNING id, user_id, name, public_key, last_seen_at
            "#,
        )
        .bind(user_id.0)
        .bind(name)
        .bind(token_hash)
        .bind(public_key)
        .fetch_one(self.pool.pg())
        .await?;
        Ok(row_to_device(row))
    }

    pub async fn get_active_by_token_hash(&self, token_hash: &[u8]) -> Result<Option<Device>> {
        let row: Option<DeviceRow> = sqlx::query_as(
            r#"
            SELECT d.id, d.user_id, d.name, d.public_key, d.last_seen_at
            FROM   devices d
            JOIN   users u ON u.id = d.user_id
            WHERE  d.token_hash = $1
              AND  d.revoked_at IS NULL
              AND  u.revoked_at IS NULL
            "#,
        )
        .bind(token_hash)
        .fetch_optional(self.pool.pg())
        .await?;
        Ok(row.map(row_to_device))
    }

    pub async fn touch_last_seen(&self, id: DeviceId) -> Result<()> {
        sqlx::query("UPDATE devices SET last_seen_at = now() WHERE id = $1")
            .bind(id.0)
            .execute(self.pool.pg())
            .await?;
        Ok(())
    }

    pub async fn revoke(&self, id: DeviceId) -> Result<()> {
        sqlx::query("UPDATE devices SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL")
            .bind(id.0)
            .execute(self.pool.pg())
            .await?;
        Ok(())
    }

    pub async fn list_for_user(&self, user_id: UserId) -> Result<Vec<Device>> {
        let rows: Vec<DeviceRow> = sqlx::query_as(
            r#"
            SELECT id, user_id, name, public_key, last_seen_at
            FROM   devices
            WHERE  user_id = $1 AND revoked_at IS NULL
            ORDER BY name
            "#,
        )
        .bind(user_id.0)
        .fetch_all(self.pool.pg())
        .await?;
        Ok(rows.into_iter().map(row_to_device).collect())
    }
}
