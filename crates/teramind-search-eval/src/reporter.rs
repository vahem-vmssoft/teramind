//! Per-query and aggregated metric computation + scorecard emission.

use crate::metrics::{mrr, ndcg_at_k, precision_at_k, recall_at_k};
use crate::types::{CorpusSize, EvalReport, MetricsRow, QueryClass};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct PerQuery {
    pub id: String,
    pub class: QueryClass,
    pub relevance: Vec<u32>,
    pub total_relevant: u32,
}

pub fn aggregate(per_query: &[PerQuery], size: CorpusSize, p95_ms: u32) -> EvalReport {
    let overall = row_for(per_query.iter());
    let mut by_class: BTreeMap<QueryClass, MetricsRow> = BTreeMap::new();
    for class in QueryClass::all() {
        let slice = per_query.iter().filter(|p| p.class == *class);
        by_class.insert(*class, row_for(slice));
    }
    EvalReport {
        overall,
        by_class,
        query_latency_p95_ms: p95_ms,
        corpus_size: size,
    }
}

fn row_for<'a, I: Iterator<Item = &'a PerQuery>>(iter: I) -> MetricsRow {
    let mut n = 0usize;
    let mut ndcg = 0.0;
    let mut mrr_sum = 0.0;
    let mut p5 = 0.0;
    let mut p10 = 0.0;
    let mut r10 = 0.0;
    for p in iter {
        n += 1;
        ndcg += ndcg_at_k(&p.relevance, 10);
        mrr_sum += mrr(&p.relevance);
        p5 += precision_at_k(&p.relevance, 5);
        p10 += precision_at_k(&p.relevance, 10);
        r10 += recall_at_k(&p.relevance, 10, p.total_relevant);
    }
    if n == 0 {
        return MetricsRow {
            n_queries: 0,
            ndcg_at_10: 0.0,
            mrr: 0.0,
            p_at_5: 0.0,
            p_at_10: 0.0,
            r_at_10: 0.0,
        };
    }
    let denom = n as f64;
    MetricsRow {
        n_queries: n,
        ndcg_at_10: ndcg / denom,
        mrr: mrr_sum / denom,
        p_at_5: p5 / denom,
        p_at_10: p10 / denom,
        r_at_10: r10 / denom,
    }
}

pub fn write_results(dir: &Path, report: &EvalReport) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(dir.join("eval-results.json"), json)?;
    std::fs::write(dir.join("eval-scorecard.md"), render_markdown(report))?;
    Ok(())
}

pub fn render_markdown(report: &EvalReport) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    writeln!(s, "# Teramind Search Eval Scorecard\n").unwrap();
    writeln!(
        s,
        "Corpus: {} sessions / {} turns / {} tool calls / {} diffs",
        report.corpus_size.sessions,
        report.corpus_size.turns,
        report.corpus_size.tool_calls,
        report.corpus_size.file_diffs,
    )
    .unwrap();
    writeln!(
        s,
        "p95 latency per query: {} ms\n",
        report.query_latency_p95_ms
    )
    .unwrap();

    writeln!(s, "## Overall\n").unwrap();
    writeln!(s, "| n | nDCG@10 | MRR | P@5 | P@10 | R@10 |").unwrap();
    writeln!(s, "|---|---:|---:|---:|---:|---:|").unwrap();
    write_row(&mut s, &report.overall);

    writeln!(s, "\n## Per class\n").unwrap();
    writeln!(s, "| Class | n | nDCG@10 | MRR | P@5 | P@10 | R@10 |").unwrap();
    writeln!(s, "|---|---|---:|---:|---:|---:|---:|").unwrap();
    for (class, row) in &report.by_class {
        write!(s, "| {:?} ", class).unwrap();
        writeln!(
            s,
            "| {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |",
            row.n_queries, row.ndcg_at_10, row.mrr, row.p_at_5, row.p_at_10, row.r_at_10,
        )
        .unwrap();
    }
    s
}

fn write_row(s: &mut String, row: &MetricsRow) {
    use std::fmt::Write;
    writeln!(
        s,
        "| {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |",
        row.n_queries, row.ndcg_at_10, row.mrr, row.p_at_5, row.p_at_10, row.r_at_10,
    )
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_overall_averages_metrics() {
        let pq = vec![
            PerQuery {
                id: "a".into(),
                class: QueryClass::NaturalLanguage,
                relevance: vec![2, 0, 0, 1],
                total_relevant: 3,
            },
            PerQuery {
                id: "b".into(),
                class: QueryClass::NaturalLanguage,
                relevance: vec![0, 1, 2, 0],
                total_relevant: 3,
            },
        ];
        let size = CorpusSize {
            sessions: 0,
            turns: 0,
            tool_calls: 0,
            file_diffs: 0,
        };
        let r = aggregate(&pq, size, 0);
        assert_eq!(r.overall.n_queries, 2);
        assert!(r.overall.ndcg_at_10 > 0.0 && r.overall.ndcg_at_10 < 1.0);
        assert!((r.overall.mrr - 0.75).abs() < 1e-9);
    }

    #[test]
    fn aggregate_zero_rows_for_classes_without_queries() {
        let pq: Vec<PerQuery> = Vec::new();
        let size = CorpusSize {
            sessions: 0,
            turns: 0,
            tool_calls: 0,
            file_diffs: 0,
        };
        let r = aggregate(&pq, size, 0);
        assert_eq!(r.overall.n_queries, 0);
        for class in QueryClass::all() {
            assert_eq!(r.by_class[class].n_queries, 0);
        }
    }

    #[test]
    fn markdown_contains_per_class_rows() {
        let pq = vec![PerQuery {
            id: "a".into(),
            class: QueryClass::CodeSnippet,
            relevance: vec![1],
            total_relevant: 1,
        }];
        let r = aggregate(
            &pq,
            CorpusSize {
                sessions: 1,
                turns: 1,
                tool_calls: 0,
                file_diffs: 0,
            },
            5,
        );
        let md = render_markdown(&r);
        assert!(md.contains("Overall"));
        assert!(md.contains("Per class"));
        assert!(md.contains("CodeSnippet"));
    }
}
