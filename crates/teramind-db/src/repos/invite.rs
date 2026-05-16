use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::{DeviceId, InviteId};
use time::OffsetDateTime;
use uuid::Uuid;

type InviteRow = (Uuid, String, Option<String>, Option<String>,
                  OffsetDateTime, OffsetDateTime, Option<OffsetDateTime>);

fn row_to_invite(r: InviteRow) -> Invite {
    Invite {
        id: InviteId(r.0), invited_email: r.1, display_name: r.2,
        created_by: r.3, created_at: r.4, expires_at: r.5, redeemed_at: r.6,
    }
}

#[derive(Clone)]
pub struct InviteRepo { pool: DbPool }

#[derive(Debug, Clone)]
pub struct Invite {
    pub id: InviteId,
    pub invited_email: String,
    pub display_name: Option<String>,
    pub created_by: Option<String>,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub redeemed_at: Option<OffsetDateTime>,
}

impl InviteRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    pub async fn create(
        &self,
        code_hash: &[u8],
        invited_email: &str,
        display_name: Option<&str>,
        created_by: Option<&str>,
        expires_at: OffsetDateTime,
    ) -> Result<InviteId> {
        let row: (Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO invites (code_hash, invited_email, display_name, created_by, expires_at)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id
            "#)
            .bind(code_hash).bind(invited_email).bind(display_name)
            .bind(created_by).bind(expires_at)
            .fetch_one(self.pool.pg()).await?;
        Ok(InviteId(row.0))
    }

    /// Look up an invite by hash regardless of redemption/expiry status.
    pub async fn find_by_hash(&self, code_hash: &[u8]) -> Result<Option<Invite>> {
        let row: Option<InviteRow> = sqlx::query_as(
            r#"
            SELECT id, invited_email, display_name, created_by,
                   created_at, expires_at, redeemed_at
            FROM   invites
            WHERE  code_hash = $1
            "#)
            .bind(code_hash).fetch_optional(self.pool.pg()).await?;
        Ok(row.map(row_to_invite))
    }

    pub async fn find_redeemable(&self, code_hash: &[u8]) -> Result<Option<Invite>> {
        let row: Option<InviteRow> = sqlx::query_as(
            r#"
            SELECT id, invited_email, display_name, created_by,
                   created_at, expires_at, redeemed_at
            FROM   invites
            WHERE  code_hash   = $1
              AND  redeemed_at IS NULL
              AND  expires_at  > now()
            "#)
            .bind(code_hash).fetch_optional(self.pool.pg()).await?;
        Ok(row.map(row_to_invite))
    }

    /// Marks the invite redeemed. Returns rows_affected — 1 on success, 0 on
    /// race (someone else redeemed first). Caller treats 0 as a 409.
    pub async fn mark_redeemed(&self, code_hash: &[u8], device_id: DeviceId) -> Result<u64> {
        let r = sqlx::query(
            r#"
            UPDATE invites
            SET    redeemed_at = now(), redeemed_device = $2
            WHERE  code_hash = $1 AND redeemed_at IS NULL AND expires_at > now()
            "#)
            .bind(code_hash).bind(device_id.0)
            .execute(self.pool.pg()).await?;
        Ok(r.rows_affected())
    }

    pub async fn list_outstanding(&self) -> Result<Vec<Invite>> {
        let rows: Vec<InviteRow> = sqlx::query_as(
            r#"
            SELECT id, invited_email, display_name, created_by,
                   created_at, expires_at, redeemed_at
            FROM   invites
            WHERE  redeemed_at IS NULL AND expires_at > now()
            ORDER  BY created_at DESC
            "#).fetch_all(self.pool.pg()).await?;
        Ok(rows.into_iter().map(row_to_invite).collect())
    }

    pub async fn revoke(&self, id: InviteId) -> Result<()> {
        sqlx::query("UPDATE invites SET expires_at = now() WHERE id = $1 AND redeemed_at IS NULL")
            .bind(id.0).execute(self.pool.pg()).await?;
        Ok(())
    }
}
