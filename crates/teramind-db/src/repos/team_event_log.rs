use crate::error::Result;
use crate::pool::DbPool;
use serde_json::Value;
use teramind_core::ids::UserId;
use time::OffsetDateTime;
use uuid::Uuid;

type EventRow = (
    Uuid,
    String,
    Option<Uuid>,
    Option<String>,
    Value,
    OffsetDateTime,
);

fn row_to_event(r: EventRow) -> TeamEventRow {
    TeamEventRow {
        id: r.0,
        kind: r.1,
        user_id: r.2.map(UserId),
        cwd: r.3,
        payload: r.4,
        ts: r.5,
    }
}

#[derive(Debug, Clone)]
pub struct TeamEventRow {
    pub id: Uuid,
    pub kind: String,
    pub user_id: Option<UserId>,
    pub cwd: Option<String>,
    pub payload: Value,
    pub ts: OffsetDateTime,
}

#[derive(Clone)]
pub struct TeamEventLogRepo {
    pool: DbPool,
}

impl TeamEventLogRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn insert(
        &self,
        kind: &str,
        user_id: Option<UserId>,
        cwd: Option<String>,
        payload: Value,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO team_event_log (kind, user_id, cwd, payload)
               VALUES ($1, $2, $3, $4)"#,
        )
        .bind(kind)
        .bind(user_id.map(|u| u.0))
        .bind(cwd)
        .bind(payload)
        .execute(self.pool.pg())
        .await?;
        Ok(())
    }

    pub async fn list_recent(
        &self,
        kind: Option<&str>,
        before: Option<OffsetDateTime>,
        user_id: Option<UserId>,
        limit: i64,
    ) -> Result<Vec<TeamEventRow>> {
        let rows: Vec<EventRow> = sqlx::query_as(
            r#"SELECT id, kind, user_id, cwd, payload, ts
               FROM team_event_log
               WHERE ($1::text IS NULL OR kind = $1)
                 AND ($2::timestamptz IS NULL OR ts < $2)
                 AND ($3::uuid IS NULL OR user_id = $3)
               ORDER BY ts DESC
               LIMIT $4"#,
        )
        .bind(kind)
        .bind(before)
        .bind(user_id.map(|u| u.0))
        .bind(limit)
        .fetch_all(self.pool.pg())
        .await?;
        Ok(rows.into_iter().map(row_to_event).collect())
    }

    pub async fn prune_older_than(&self, days: i64) -> Result<u64> {
        let r = sqlx::query(
            r#"DELETE FROM team_event_log WHERE ts < now() - ($1::int * interval '1 day')"#,
        )
        .bind(days as i32)
        .execute(self.pool.pg())
        .await?;
        Ok(r.rows_affected())
    }
}
