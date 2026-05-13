use crate::error::Result;
use crate::pool::DbPool;

#[derive(Clone)]
pub struct StorageStatsRepo {
    pool: DbPool,
}

pub struct Sample {
    pub pg_bytes: i64,
    pub jsonl_bytes: i64,
    pub session_count: i64,
    pub turn_count: i64,
    pub diff_count: i64,
}

impl StorageStatsRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
    pub async fn insert(&self, s: Sample) -> Result<()> {
        sqlx::query("INSERT INTO storage_stats (pg_bytes, jsonl_bytes, session_count, turn_count, diff_count) VALUES ($1,$2,$3,$4,$5)")
            .bind(s.pg_bytes).bind(s.jsonl_bytes).bind(s.session_count).bind(s.turn_count).bind(s.diff_count)
            .execute(self.pool.pg()).await?;
        Ok(())
    }
    pub async fn count_sessions(&self) -> Result<i64> {
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM sessions")
            .fetch_one(self.pool.pg())
            .await?;
        Ok(n)
    }
    pub async fn count_turns(&self) -> Result<i64> {
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM turns")
            .fetch_one(self.pool.pg())
            .await?;
        Ok(n)
    }
    pub async fn count_diffs(&self) -> Result<i64> {
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM file_diffs")
            .fetch_one(self.pool.pg())
            .await?;
        Ok(n)
    }
    pub async fn pg_database_size(&self, database: &str) -> Result<i64> {
        let (n,): (i64,) = sqlx::query_as("SELECT pg_database_size($1)::bigint")
            .bind(database)
            .fetch_one(self.pool.pg())
            .await?;
        Ok(n)
    }
}
