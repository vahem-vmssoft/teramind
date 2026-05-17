//! Eval harness: load corpus into a throwaway PG, run every query through
//! `SearchRepo::fts_turns` + `trgm_diffs`, capture ranked hit IDs.

use crate::corpus;
use crate::queries_bank::QUERIES;
use crate::reporter;
use crate::types::CorpusSize;
use std::path::Path;
use std::time::Instant;
use teramind_db::repos::SearchRepo;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};

pub async fn run(
    corpus_root: &Path,
    out_dir: &Path,
    semantic: bool,
    semantic_weight: f32,
) -> anyhow::Result<()> {
    if semantic {
        crate::semantic::run_with_semantic(corpus_root, out_dir, semantic_weight).await
    } else {
        run_lexical(corpus_root, out_dir).await
    }
}

async fn run_lexical(corpus_root: &Path, out_dir: &Path) -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let sup = PgSupervisor::start(tmp.path().join("pgdata"), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let cor = corpus::load(corpus_root)?;
    let size = CorpusSize {
        sessions: cor.sessions.len() as u32,
        turns: cor.turns.len() as u32,
        tool_calls: cor.tool_calls.len() as u32,
        file_diffs: cor.file_diffs.len() as u32,
    };
    corpus::ingest(&pool, &cor).await?;

    let search = SearchRepo::new(pool.clone());
    let qrels = load_qrels(corpus_root)?;

    let mut per_query: Vec<reporter::PerQuery> = Vec::with_capacity(QUERIES.len());
    let mut latencies: Vec<u128> = Vec::with_capacity(QUERIES.len());
    for q in QUERIES {
        let started = Instant::now();
        let fts = search.fts_turns(q.text, 10).await.unwrap_or_default();
        let diffs = search.trgm_diffs(q.text, 10).await.unwrap_or_default();
        let elapsed = started.elapsed().as_millis();
        latencies.push(elapsed);

        let mut hit_ids: Vec<String> = Vec::with_capacity(20);
        for f in &fts {
            hit_ids.push(format!("turn:{}", f.turn_id));
        }
        for d in &diffs {
            hit_ids.push(format!("diff:{}", d.diff_id));
        }

        let relevance = relevance_for(&qrels, q.id, &hit_ids);
        let total_rel = qrels
            .judgments
            .get(q.id)
            .map(|v| v.iter().filter(|j| j.grade > 0).count() as u32)
            .unwrap_or(0);

        per_query.push(reporter::PerQuery {
            id: q.id.into(),
            class: q.class,
            relevance,
            total_relevant: total_rel,
        });
    }
    sup.shutdown().await?;

    latencies.sort();
    let p95_ms = percentile_u32(&latencies, 95);

    let report = reporter::aggregate(&per_query, size, p95_ms);
    reporter::write_results(out_dir, &report)?;
    println!(
        "teramind-search-eval: nDCG@10={:.3}  MRR={:.3}  p95={}ms  ({} queries)",
        report.overall.ndcg_at_10,
        report.overall.mrr,
        report.query_latency_p95_ms,
        report.overall.n_queries,
    );
    Ok(())
}

pub(crate) fn load_qrels(root: &Path) -> anyhow::Result<crate::types::QrelsFile> {
    let path = root.join("qrels.toml");
    if !path.exists() {
        return Ok(Default::default());
    }
    let body = std::fs::read_to_string(&path)?;
    Ok(toml::from_str(&body)?)
}

pub(crate) fn relevance_for(
    qrels: &crate::types::QrelsFile,
    qid: &str,
    hit_ids: &[String],
) -> Vec<u32> {
    let judgments = match qrels.judgments.get(qid) {
        Some(v) => v,
        None => return hit_ids.iter().map(|_| 0).collect(),
    };
    let map: std::collections::HashMap<&str, u32> = judgments
        .iter()
        .map(|j| (j.item.as_str(), j.grade))
        .collect();
    hit_ids
        .iter()
        .map(|h| *map.get(h.as_str()).unwrap_or(&0))
        .collect()
}

pub(crate) fn percentile_u32(sorted_ms: &[u128], pct: u32) -> u32 {
    if sorted_ms.is_empty() {
        return 0;
    }
    let idx = ((sorted_ms.len() as f64) * (pct as f64) / 100.0).ceil() as usize;
    let idx = idx.min(sorted_ms.len() - 1);
    sorted_ms[idx] as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relevance_for_returns_zero_for_unjudged_query() {
        let qrels = crate::types::QrelsFile::default();
        let r = relevance_for(&qrels, "nl-99", &["turn:abc".into(), "diff:def".into()]);
        assert_eq!(r, vec![0, 0]);
    }

    #[test]
    fn percentile_handles_short_inputs() {
        assert_eq!(percentile_u32(&[1, 2, 3, 4], 95), 4);
        assert_eq!(percentile_u32(&[], 95), 0);
    }
}
