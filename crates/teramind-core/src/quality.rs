//! Shared between teramind-search-eval (writer) and teramind-sync-server (reader).

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityRunOutput {
    pub baseline_label: String,
    pub model: Option<String>,
    pub ndcg10: f64,
    pub mrr: f64,
    pub precision_5: f64,
    pub precision_10: f64,
    pub recall_10: f64,
    pub p50_latency_ms: f64,
    pub p95_latency_ms: f64,
    pub query_count: u32,
    pub corpus_size: u32,
    pub per_class: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn round_trips_through_json() {
        let q = QualityRunOutput {
            baseline_label: "lexical".into(),
            model: None,
            ndcg10: 0.142,
            mrr: 0.301,
            precision_5: 0.23,
            precision_10: 0.18,
            recall_10: 0.42,
            p50_latency_ms: 42.0,
            p95_latency_ms: 380.0,
            query_count: 100,
            corpus_size: 500,
            per_class: json!({}),
        };
        let s = serde_json::to_string(&q).unwrap();
        let back: QualityRunOutput = serde_json::from_str(&s).unwrap();
        assert_eq!(back.baseline_label, "lexical");
        assert!((back.ndcg10 - 0.142).abs() < 1e-9);
    }
}
