//! Cron-driven runner for teramind-search-eval. Persists results to quality_runs.

use crate::config::QualityConfig;
use cron::Schedule;
use std::str::FromStr;
use std::time::Duration as StdDuration;
use teramind_core::quality::QualityRunOutput;
use teramind_db::pool::DbPool;
use teramind_db::repos::QualityRunRepo;
use tokio::process::Command;
use tracing::{info, warn};

pub fn spawn(pool: DbPool, cfg: QualityConfig) -> Option<tokio::task::JoinHandle<()>> {
    if !cfg.enabled {
        return None;
    }
    let cron = cfg.cron.clone().unwrap_or_else(|| "0 2 * * *".into());
    let schedule = match Schedule::from_str(&format!("0 {cron}")) {
        // cron crate expects 6-field (sec, min, hr, dom, mon, dow).
        // We prepend "0" so users can supply 5-field "min hr dom mon dow".
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "invalid cron in [quality]; disabling scheduler");
            return None;
        }
    };
    let repo = QualityRunRepo::new(pool);
    Some(tokio::spawn(
        async move { run_loop(repo, cfg, schedule).await },
    ))
}

async fn run_loop(repo: QualityRunRepo, cfg: QualityConfig, schedule: Schedule) {
    use chrono::Utc;
    let mut last_for: std::collections::HashMap<String, std::time::Instant> = Default::default();
    while let Some(next) = schedule.upcoming(Utc).next() {
        let now = Utc::now();
        let delay = (next - now).to_std().unwrap_or(StdDuration::from_secs(60));
        tokio::time::sleep(delay).await;

        for baseline in &cfg.baselines {
            // Single-flight per baseline.
            if let Some(t) = last_for.get(baseline) {
                if t.elapsed() < StdDuration::from_secs(60) {
                    continue;
                }
            }
            last_for.insert(baseline.clone(), std::time::Instant::now());
            run_one(&repo, &cfg.eval_binary, baseline).await;
        }
    }
}

/// Helper that runs a single eval baseline and persists a `quality_runs` row.
///
/// Always inserts a row — successes carry real metrics; failures (binary
/// missing, non-zero exit, JSON parse error) carry sentinel `0.0` metrics
/// and an `error` field in `raw_json`. Dashboard §6 requires failures be
/// observable as rows, not just log lines.
pub async fn run_one(repo: &QualityRunRepo, binary: &str, baseline: &str) {
    info!(baseline, "starting scheduled eval");
    // The eval CLI surface: `teramind-search-eval run --baseline-label <label> --json`
    let out = Command::new(binary)
        .arg("run")
        .arg("--baseline-label")
        .arg(baseline)
        .arg("--json")
        .output()
        .await;
    match out {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            match serde_json::from_str::<QualityRunOutput>(&stdout) {
                Ok(q) => {
                    let raw = serde_json::to_value(&q).unwrap_or(serde_json::json!({}));
                    let res = repo
                        .insert(
                            &q.baseline_label,
                            q.model.clone(),
                            q.ndcg10,
                            q.mrr,
                            q.precision_5,
                            q.precision_10,
                            q.recall_10,
                            q.p50_latency_ms,
                            q.p95_latency_ms,
                            q.query_count as i32,
                            q.corpus_size as i32,
                            q.per_class.clone(),
                            raw,
                            "scheduled",
                        )
                        .await;
                    if let Err(e) = res {
                        warn!(error = %e, "quality_runs insert failed");
                    }
                }
                Err(e) => {
                    warn!(error = %e, "failed to parse eval output");
                    let raw = serde_json::json!({ "error": e.to_string(), "stdout": stdout });
                    persist_failure(repo, baseline, raw).await;
                }
            }
        }
        Ok(o) => {
            warn!(status = ?o.status, "eval binary returned non-zero");
            let raw = serde_json::json!({
                "error": format!("eval binary returned non-zero: {:?}", o.status),
                "stderr": String::from_utf8_lossy(&o.stderr).into_owned(),
            });
            persist_failure(repo, baseline, raw).await;
        }
        Err(e) => {
            warn!(error = %e, baseline, "eval binary failed to spawn");
            let raw = serde_json::json!({
                "error": format!("eval binary failed to spawn: {e}"),
            });
            persist_failure(repo, baseline, raw).await;
        }
    }
}

async fn persist_failure(
    repo: &QualityRunRepo,
    baseline: &str,
    raw_json: serde_json::Value,
) {
    // Sentinel metrics (0.0) for failures — NaN would violate downstream
    // validators that require finite f64 in the upload path.
    if let Err(e) = repo
        .insert(
            baseline,
            None,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0,
            0,
            serde_json::json!({}),
            raw_json,
            "scheduled",
        )
        .await
    {
        warn!(error = %e, "quality_runs failure-row insert failed");
    }
}
