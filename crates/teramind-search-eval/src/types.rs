//! Query/qrels/report types shared by the generator, harness, reporter,
//! and gate modules.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryClass {
    NaturalLanguage,
    StackTrace,
    CodeSnippet,
    ToolTyped,
    SymbolicPath,
}

impl QueryClass {
    pub fn all() -> &'static [QueryClass] {
        use QueryClass::*;
        &[NaturalLanguage, StackTrace, CodeSnippet, ToolTyped, SymbolicPath]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Query {
    pub id: String,
    pub class: QueryClass,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QueriesFile {
    pub queries: Vec<Query>,
}

/// `qrels.toml` shape: per-query, a list of (item_id, grade) tuples.
/// Item id encodes the hit kind: "turn:<uuid>", "tool:<uuid>", "diff:<uuid>", "skill:<uuid>".
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QrelsFile {
    pub judgments: BTreeMap<String, Vec<Judgment>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Judgment {
    pub item: String,
    pub grade: u32,
}

/// One row of metrics — either a per-class slice or the overall row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricsRow {
    pub n_queries: usize,
    pub ndcg_at_10: f64,
    pub mrr: f64,
    pub p_at_5: f64,
    pub p_at_10: f64,
    pub r_at_10: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvalReport {
    pub overall: MetricsRow,
    pub by_class: BTreeMap<QueryClass, MetricsRow>,
    /// p95 latency per single-query execution (milliseconds).
    pub query_latency_p95_ms: u32,
    pub corpus_size: CorpusSize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CorpusSize {
    pub sessions: u32,
    pub turns: u32,
    pub tool_calls: u32,
    pub file_diffs: u32,
}

/// `baseline.json` is structurally identical to `EvalReport`.
pub type Baseline = EvalReport;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_file_roundtrips_through_toml() {
        let qf = QueriesFile {
            queries: vec![Query {
                id: "nl-1".into(),
                class: QueryClass::NaturalLanguage,
                text: "how did we fix the JWT bug".into(),
            }],
        };
        let s = toml::to_string(&qf).unwrap();
        let back: QueriesFile = toml::from_str(&s).unwrap();
        assert_eq!(back.queries.len(), 1);
        assert_eq!(back.queries[0].id, "nl-1");
        assert!(matches!(back.queries[0].class, QueryClass::NaturalLanguage));
    }

    #[test]
    fn qrels_file_roundtrips_through_toml() {
        let mut judgments = BTreeMap::new();
        judgments.insert(
            "nl-1".into(),
            vec![Judgment { item: "turn:abc".into(), grade: 2 }],
        );
        let qrels = QrelsFile { judgments };
        let s = toml::to_string(&qrels).unwrap();
        let back: QrelsFile = toml::from_str(&s).unwrap();
        assert_eq!(back.judgments.get("nl-1").unwrap()[0].grade, 2);
    }

    #[test]
    fn eval_report_roundtrips_through_json() {
        let mut by_class = BTreeMap::new();
        by_class.insert(QueryClass::NaturalLanguage, MetricsRow {
            n_queries: 20, ndcg_at_10: 0.8, mrr: 0.7, p_at_5: 0.6, p_at_10: 0.5, r_at_10: 0.4,
        });
        let report = EvalReport {
            overall: MetricsRow {
                n_queries: 100, ndcg_at_10: 0.75, mrr: 0.6, p_at_5: 0.55, p_at_10: 0.5, r_at_10: 0.45,
            },
            by_class,
            query_latency_p95_ms: 250,
            corpus_size: CorpusSize { sessions: 500, turns: 2500, tool_calls: 5000, file_diffs: 500 },
        };
        let j = serde_json::to_string(&report).unwrap();
        let back: EvalReport = serde_json::from_str(&j).unwrap();
        assert_eq!(report, back);
    }
}
