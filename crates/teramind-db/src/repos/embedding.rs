//! Storage layer for the `embeddings` table.

use crate::error::Result;
use crate::pool::DbPool;
use pgvector::Vector;
use uuid::Uuid;

#[derive(Clone)]
pub struct EmbeddingRepo {
    pool: DbPool,
}

#[derive(Debug, Clone)]
pub struct ToEmbedRow {
    pub kind: String,
    pub item_id: Uuid,
    pub text: String,
}

impl EmbeddingRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn fetch_to_embed(&self, model: &str, limit: u32) -> Result<Vec<ToEmbedRow>> {
        let rows: Vec<(String, Uuid, String)> = sqlx::query_as(
            r#"
            SELECT v.kind, v.item_id, v.text
            FROM   traces_to_embed v
            WHERE  NOT EXISTS (
                SELECT 1 FROM embeddings e
                WHERE  e.item_kind = v.kind
                  AND  e.item_id   = v.item_id
                  AND  e.model     = $1
            )
            LIMIT  $2
            "#,
        )
        .bind(model)
        .bind(limit as i64)
        .fetch_all(self.pool.pg())
        .await?;
        Ok(rows
            .into_iter()
            .map(|(kind, item_id, text)| ToEmbedRow { kind, item_id, text })
            .collect())
    }

    pub async fn bulk_insert(
        &self,
        rows: &[ToEmbedRow],
        model: &str,
        dim: i32,
        vectors: &[Vec<f32>],
    ) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        assert_eq!(rows.len(), vectors.len(), "ToEmbedRow/vector length mismatch");
        let mut written = 0usize;
        let mut tx = self.pool.pg().begin().await?;
        for (row, vec) in rows.iter().zip(vectors.iter()) {
            let v = Vector::from(vec.clone());
            let r = sqlx::query(
                r#"
                INSERT INTO embeddings (item_kind, item_id, model, dim, embedding)
                VALUES ($1, $2, $3, $4, $5)
                ON CONFLICT (item_kind, item_id, model) DO NOTHING
                "#,
            )
            .bind(&row.kind)
            .bind(row.item_id)
            .bind(model)
            .bind(dim)
            .bind(v)
            .execute(&mut *tx)
            .await?;
            written += r.rows_affected() as usize;
        }
        tx.commit().await?;
        Ok(written)
    }

    pub async fn backlog(&self, model: &str) -> Result<i64> {
        let (n,): (i64,) = sqlx::query_as(
            r#"
            SELECT count(*) FROM traces_to_embed v
            WHERE NOT EXISTS (
                SELECT 1 FROM embeddings e
                WHERE  e.item_kind = v.kind
                  AND  e.item_id   = v.item_id
                  AND  e.model     = $1
            )
            "#,
        )
        .bind(model)
        .fetch_one(self.pool.pg())
        .await?;
        Ok(n)
    }

    pub async fn sweep_orphans(&self) -> Result<u64> {
        let r = sqlx::query(
            r#"
            DELETE FROM embeddings e
            WHERE (e.item_kind = 'turn'
                   AND NOT EXISTS (SELECT 1 FROM turns t WHERE t.id = e.item_id))
               OR (e.item_kind = 'file_diff'
                   AND NOT EXISTS (SELECT 1 FROM file_diffs d WHERE d.id = e.item_id))
            "#,
        )
        .execute(self.pool.pg())
        .await?;
        Ok(r.rows_affected())
    }
}
