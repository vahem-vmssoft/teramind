use crate::error::Result;
use crate::pool::DbPool;
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

type QualityRow = (
    Uuid,
    String,
    Option<String>,
    f64,
    f64,
    f64,
    f64,
    f64,
    f64,
    f64,
    i32,
    i32,
    Value,
    Value,
    OffsetDateTime,
    String,
);

fn row_to_quality(r: QualityRow) -> QualityRunRow {
    QualityRunRow {
        id: r.0,
        baseline_label: r.1,
        model: r.2,
        ndcg10: r.3,
        mrr: r.4,
        precision_5: r.5,
        precision_10: r.6,
        recall_10: r.7,
        p50_latency_ms: r.8,
        p95_latency_ms: r.9,
        query_count: r.10,
        corpus_size: r.11,
        per_class: r.12,
        raw_json: r.13,
        ran_at: r.14,
        source: r.15,
    }
}

#[derive(Debug, Clone)]
pub struct QualityRunRow {
    pub id: Uuid,
    pub baseline_label: String,
    pub model: Option<String>,
    pub ndcg10: f64,
    pub mrr: f64,
    pub precision_5: f64,
    pub precision_10: f64,
    pub recall_10: f64,
    pub p50_latency_ms: f64,
    pub p95_latency_ms: f64,
    pub query_count: i32,
    pub corpus_size: i32,
    pub per_class: Value,
    pub raw_json: Value,
    pub ran_at: OffsetDateTime,
    pub source: String,
}

#[derive(Clone)]
pub struct QualityRunRepo {
    pool: DbPool,
}

impl QualityRunRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert(
        &self,
        baseline_label: &str,
        model: Option<String>,
        ndcg10: f64,
        mrr: f64,
        p5: f64,
        p10: f64,
        r10: f64,
        p50: f64,
        p95: f64,
        query_count: i32,
        corpus_size: i32,
        per_class: Value,
        raw_json: Value,
        source: &str,
    ) -> Result<Uuid> {
        let row: (Uuid,) = sqlx::query_as(
            r#"INSERT INTO quality_runs
               (baseline_label, model, ndcg10, mrr, precision_5, precision_10, recall_10,
                p50_latency_ms, p95_latency_ms, query_count, corpus_size,
                per_class, raw_json, source)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
               RETURNING id"#,
        )
        .bind(baseline_label)
        .bind(model)
        .bind(ndcg10)
        .bind(mrr)
        .bind(p5)
        .bind(p10)
        .bind(r10)
        .bind(p50)
        .bind(p95)
        .bind(query_count)
        .bind(corpus_size)
        .bind(per_class)
        .bind(raw_json)
        .bind(source)
        .fetch_one(self.pool.pg())
        .await?;
        Ok(row.0)
    }

    pub async fn list_recent(
        &self,
        baseline: Option<&str>,
        limit: i64,
    ) -> Result<Vec<QualityRunRow>> {
        let rows: Vec<QualityRow> = sqlx::query_as(
            r#"SELECT id, baseline_label, model, ndcg10, mrr, precision_5, precision_10, recall_10,
                       p50_latency_ms, p95_latency_ms, query_count, corpus_size,
                       per_class, raw_json, ran_at, source
               FROM quality_runs
               WHERE ($1::text IS NULL OR baseline_label = $1)
               ORDER BY ran_at DESC
               LIMIT $2"#,
        )
        .bind(baseline)
        .bind(limit)
        .fetch_all(self.pool.pg())
        .await?;
        Ok(rows.into_iter().map(row_to_quality).collect())
    }

    pub async fn latest(&self, baseline_label: &str) -> Result<Option<QualityRunRow>> {
        let row: Option<QualityRow> = sqlx::query_as(
            r#"SELECT id, baseline_label, model, ndcg10, mrr, precision_5, precision_10, recall_10,
                       p50_latency_ms, p95_latency_ms, query_count, corpus_size,
                       per_class, raw_json, ran_at, source
               FROM quality_runs
               WHERE baseline_label = $1
               ORDER BY ran_at DESC LIMIT 1"#,
        )
        .bind(baseline_label)
        .fetch_optional(self.pool.pg())
        .await?;
        Ok(row.map(row_to_quality))
    }
}
