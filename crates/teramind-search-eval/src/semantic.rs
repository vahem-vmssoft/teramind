//! Semantic eval mode. Loads corpus into throwaway PG, fills embeddings,
//! runs every query with semantic enabled, writes *-semantic.{json,md}.

use crate::corpus;
use crate::queries_bank::QUERIES;
use crate::reporter;
use crate::types::CorpusSize;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use teramind_core::embed::EmbeddingProvider;
use teramind_db::repos::{EmbeddingRepo, SearchRepo};
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use tracing::info;

pub async fn run_with_semantic(
    corpus_root: &Path,
    out_dir: &Path,
    semantic_weight: f32,
) -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let sup = PgSupervisor::start(tmp.path().join("pgdata"), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let cor = corpus::load(corpus_root)?;
    let size = CorpusSize {
        sessions:   cor.sessions.len()   as u32,
        turns:      cor.turns.len()      as u32,
        tool_calls: cor.tool_calls.len() as u32,
        file_diffs: cor.file_diffs.len() as u32,
    };
    corpus::ingest(&pool, &cor).await?;

    let cfg = teramindd::config::EmbedConfig::default();
    let provider = teramindd::services::embed::build_provider(&cfg)
        .map_err(|e| anyhow::anyhow!("provider init: {e}. Is Ollama running?"))?;
    let model = format!("ollama:{}", cfg.model);

    provider.health_check().await
        .map_err(|e| anyhow::anyhow!("provider health: {e}. Is Ollama running with the model pulled?"))?;
    info!("eval-semantic: provider {} healthy", provider.model_id());

    let embed_repo = EmbeddingRepo::new(pool.clone());
    fill_all_embeddings(&embed_repo, provider.clone(), &model).await?;

    let search = SearchRepo::new(pool.clone());
    let qrels = crate::harness::load_qrels(corpus_root)?;

    let mut per_query: Vec<reporter::PerQuery> = Vec::with_capacity(QUERIES.len());
    let mut latencies: Vec<u128> = Vec::with_capacity(QUERIES.len());
    for q in QUERIES {
        let started = Instant::now();
        let q_vec = provider.embed(&[q.text.to_string()]).await
            .map(|mut v| v.pop()).ok().flatten();
        let fts = search.fts_turns(q.text, 10).await.unwrap_or_default();
        let diffs = search.trgm_diffs(q.text, 10).await.unwrap_or_default();
        let sem_turns = match q_vec.as_ref() {
            Some(v) => search.vector_search_turns(v, &model, 10).await.unwrap_or_default(),
            None => vec![],
        };
        let sem_diffs = match q_vec.as_ref() {
            Some(v) => search.vector_search_diffs(v, &model, 10).await.unwrap_or_default(),
            None => vec![],
        };
        let elapsed = started.elapsed().as_millis();
        latencies.push(elapsed);

        let mut hit_ids: Vec<String> = Vec::with_capacity(40);
        for f in &fts       { hit_ids.push(format!("turn:{}", f.turn_id)); }
        for d in &diffs     { hit_ids.push(format!("diff:{}", d.diff_id)); }
        for s in &sem_turns { hit_ids.push(format!("turn:{}", s.turn_id)); }
        for s in &sem_diffs { hit_ids.push(format!("diff:{}", s.diff_id)); }

        let relevance = crate::harness::relevance_for(&qrels, q.id, &hit_ids);
        let total_rel = qrels.judgments.get(q.id)
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
    let p95_ms = crate::harness::percentile_u32(&latencies, 95);
    let report = reporter::aggregate(&per_query, size, p95_ms);

    std::fs::create_dir_all(out_dir)?;
    std::fs::write(out_dir.join("eval-results-semantic.json"),
                   serde_json::to_string_pretty(&report)?)?;
    std::fs::write(out_dir.join("eval-scorecard-semantic.md"),
                   reporter::render_markdown(&report))?;

    let _ = semantic_weight;  // wire to blend in v1.0.1 (paraphrase corpus)

    println!(
        "teramind-search-eval (semantic): nDCG@10={:.3} MRR={:.3} p95={}ms ({} queries)",
        report.overall.ndcg_at_10,
        report.overall.mrr,
        report.query_latency_p95_ms,
        report.overall.n_queries,
    );
    Ok(())
}

async fn fill_all_embeddings(
    repo: &EmbeddingRepo,
    provider: Arc<dyn EmbeddingProvider>,
    model: &str,
) -> anyhow::Result<()> {
    loop {
        let rows = repo.fetch_to_embed(model, 32).await?;
        if rows.is_empty() { break; }
        let texts: Vec<String> = rows.iter().map(|r| r.text.clone()).collect();
        let vectors = provider.embed(&texts).await
            .map_err(|e| anyhow::anyhow!("embed: {e}"))?;
        repo.bulk_insert(&rows, model, provider.dimension() as i32, &vectors).await?;
    }
    Ok(())
}
