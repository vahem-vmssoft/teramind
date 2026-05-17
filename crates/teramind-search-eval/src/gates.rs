//! Regression-gate comparison.
//!
//! Thresholds (per spec §9.5):
//!   * nDCG@10 (overall): must not drop more than 2 pp vs baseline.
//!   * nDCG@10 (any class): must not drop more than 5 pp vs baseline.
//!   * MRR (overall): must not drop more than 0.03 absolute.
//!   * eval p95 latency: must not exceed 3000 ms per query.

use crate::types::{Baseline, EvalReport};
use std::path::Path;

pub const NDCG_OVERALL_DROP: f64 = 0.02;
pub const NDCG_PER_CLASS_DROP: f64 = 0.05;
pub const MRR_OVERALL_DROP: f64 = 0.03;
pub const P95_LATENCY_CEILING_MS: u32 = 3_000;

#[derive(Debug, PartialEq)]
pub struct GateOutcome {
    pub passed: bool,
    pub failures: Vec<String>,
}

pub fn check(report: &EvalReport, baseline: &Baseline) -> GateOutcome {
    let mut failures = Vec::new();

    let ndcg_drop = baseline.overall.ndcg_at_10 - report.overall.ndcg_at_10;
    if ndcg_drop > NDCG_OVERALL_DROP + 1e-9 {
        failures.push(format!(
            "overall nDCG@10 dropped {:.4} (limit {:.4})",
            ndcg_drop, NDCG_OVERALL_DROP,
        ));
    }
    let mrr_drop = baseline.overall.mrr - report.overall.mrr;
    if mrr_drop > MRR_OVERALL_DROP + 1e-9 {
        failures.push(format!(
            "overall MRR dropped {:.4} (limit {:.4})",
            mrr_drop, MRR_OVERALL_DROP,
        ));
    }
    for (class, bl) in &baseline.by_class {
        if let Some(rep) = report.by_class.get(class) {
            let drop = bl.ndcg_at_10 - rep.ndcg_at_10;
            if drop > NDCG_PER_CLASS_DROP + 1e-9 {
                failures.push(format!(
                    "class {:?} nDCG@10 dropped {:.4} (limit {:.4})",
                    class, drop, NDCG_PER_CLASS_DROP,
                ));
            }
        }
    }
    if report.query_latency_p95_ms > P95_LATENCY_CEILING_MS {
        failures.push(format!(
            "p95 latency {} ms exceeds ceiling {} ms",
            report.query_latency_p95_ms, P95_LATENCY_CEILING_MS,
        ));
    }
    GateOutcome {
        passed: failures.is_empty(),
        failures,
    }
}

pub fn compare(results_path: &Path, baseline_path: &Path, update: bool) -> anyhow::Result<()> {
    let results: EvalReport = serde_json::from_slice(&std::fs::read(results_path)?)?;

    if update {
        std::fs::write(baseline_path, serde_json::to_string_pretty(&results)?)?;
        println!(
            "teramind-search-eval: baseline updated -> {}",
            baseline_path.display(),
        );
        return Ok(());
    }

    if !baseline_path.exists() {
        println!("teramind-search-eval: no baseline; pass --update-baseline to seed one.",);
        return Ok(());
    }
    let baseline: Baseline = serde_json::from_slice(&std::fs::read(baseline_path)?)?;
    let outcome = check(&results, &baseline);
    if outcome.passed {
        println!("teramind-search-eval: all gates passed");
        Ok(())
    } else {
        for f in &outcome.failures {
            eprintln!("teramind-search-eval: gate failure: {f}");
        }
        anyhow::bail!("regression gate tripped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CorpusSize, MetricsRow, QueryClass};
    use std::collections::BTreeMap;

    fn report(ndcg: f64, mrr: f64, class_ndcg: f64) -> EvalReport {
        let mut by_class = BTreeMap::new();
        by_class.insert(
            QueryClass::NaturalLanguage,
            MetricsRow {
                n_queries: 20,
                ndcg_at_10: class_ndcg,
                mrr: 0.5,
                p_at_5: 0.5,
                p_at_10: 0.4,
                r_at_10: 0.3,
            },
        );
        EvalReport {
            overall: MetricsRow {
                n_queries: 100,
                ndcg_at_10: ndcg,
                mrr,
                p_at_5: 0.5,
                p_at_10: 0.4,
                r_at_10: 0.3,
            },
            by_class,
            query_latency_p95_ms: 100,
            corpus_size: CorpusSize {
                sessions: 500,
                turns: 2500,
                tool_calls: 5000,
                file_diffs: 500,
            },
        }
    }

    #[test]
    fn equal_metrics_pass() {
        let b = report(0.80, 0.70, 0.85);
        let r = report(0.80, 0.70, 0.85);
        assert!(check(&r, &b).passed);
    }

    #[test]
    fn overall_ndcg_drop_two_pp_passes_marginally() {
        let b = report(0.80, 0.70, 0.85);
        let r = report(0.78, 0.70, 0.85);
        let o = check(&r, &b);
        assert!(o.passed, "{:?}", o.failures);
    }

    #[test]
    fn overall_ndcg_drop_more_than_two_pp_fails() {
        let b = report(0.80, 0.70, 0.85);
        let r = report(0.77, 0.70, 0.85);
        let o = check(&r, &b);
        assert!(!o.passed);
        assert!(o.failures[0].contains("nDCG@10"));
    }

    #[test]
    fn class_ndcg_drop_more_than_five_pp_fails() {
        let b = report(0.80, 0.70, 0.85);
        let r = report(0.80, 0.70, 0.79);
        let o = check(&r, &b);
        assert!(!o.passed);
        assert!(o.failures[0].contains("class"));
    }

    #[test]
    fn mrr_drop_more_than_003_fails() {
        let b = report(0.80, 0.70, 0.85);
        let r = report(0.80, 0.66, 0.85);
        let o = check(&r, &b);
        assert!(!o.passed);
        assert!(o.failures[0].contains("MRR"));
    }

    #[test]
    fn p95_latency_above_ceiling_fails() {
        let b = report(0.80, 0.70, 0.85);
        let mut r = report(0.80, 0.70, 0.85);
        r.query_latency_p95_ms = 3_500;
        let o = check(&r, &b);
        assert!(!o.passed);
        assert!(o.failures[0].contains("p95"));
    }
}
