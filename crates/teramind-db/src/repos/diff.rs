use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::{FileDiffId, SessionId, TurnId};
use teramind_core::types::file_diff::Attribution;
use time::OffsetDateTime;

#[derive(Clone)]
pub struct DiffRepo {
    pool: DbPool,
}

pub struct NewFileDiff<'a> {
    pub turn_id: Option<TurnId>,
    pub session_id: SessionId,
    pub file_path: &'a str,
    pub rel_path: &'a str,
    pub attribution: Attribution,
    pub language: Option<&'a str>,
    pub pre_excerpt: &'a str,
    pub post_excerpt: &'a str,
    pub unified_diff: &'a str,
    pub pre_hash: [u8; 32],
    pub post_hash: [u8; 32],
    pub byte_size: i32,
    pub captured_at: OffsetDateTime,
}

impl DiffRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, n: NewFileDiff<'_>) -> Result<FileDiffId> {
        let attr = match n.attribution {
            Attribution::Agent => "agent",
            Attribution::Human => "human",
        };
        let r: (uuid::Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO file_diffs (turn_id, session_id, file_path, rel_path, attribution, language,
                                    pre_excerpt, post_excerpt, unified_diff, pre_hash, post_hash, byte_size, captured_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
            RETURNING id
            "#)
            .bind(n.turn_id.map(|t| t.0)).bind(n.session_id.0)
            .bind(n.file_path).bind(n.rel_path).bind(attr).bind(n.language)
            .bind(n.pre_excerpt).bind(n.post_excerpt).bind(n.unified_diff)
            .bind(&n.pre_hash[..]).bind(&n.post_hash[..]).bind(n.byte_size).bind(n.captured_at)
            .fetch_one(self.pool.pg()).await?;
        Ok(FileDiffId(r.0))
    }
}
