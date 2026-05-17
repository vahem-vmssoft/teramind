use serde::{Deserialize, Serialize};
use teramind_core::types::ingest_event::EventEnvelope;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Request {
    Status,
    Ping,
    Shutdown,
    Search(teramind_core::types::SearchRequest),
    Recall(teramind_core::types::RecallRequest),
    AutoRecall(teramind_core::types::AutoRecallRequest),
    SaveSkill(teramind_core::types::SaveSkillRequest),
    WikiLookup {
        session_id: Option<String>,
        cwd: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    Ok,
    Pong,
    Status(StatusReport),
    Error(String),
    SearchResults(teramind_core::types::SearchResults),
    SkillRef(teramind_core::types::SkillRef),
    AutoRecallDigest {
        markdown: String,
        degraded: bool,
    },
    WikiPage {
        session_id: String,
        cwd: String,
        model: String,
        content: String,
        #[serde(with = "time::serde::rfc3339")]
        generated_at: time::OffsetDateTime,
    },
    WikiNotFound,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusReport {
    pub uptime_seconds: u64,
    pub pg_connected: bool,
    pub ingest_queue_depth: u32,
    pub ingest_drops_total: u64,
    pub last_storage_pg_bytes: i64,
    pub last_storage_jsonl_bytes: i64,
    #[serde(default)]
    pub fs_watcher_gaps_total: u64,
    #[serde(default)]
    pub embedding_provider: Option<String>,
    #[serde(default)]
    pub embedding_healthy: Option<bool>,
    #[serde(default)]
    pub embedding_backlog: Option<i64>,
    #[serde(default)]
    pub embedding_last_filled_unix: Option<u64>,
    #[serde(default)]
    pub summary_provider: Option<String>,
    #[serde(default)]
    pub summary_healthy: Option<bool>,
    #[serde(default)]
    pub summary_backlog: Option<i64>,
    #[serde(default)]
    pub summary_written_total: Option<u64>,
    #[serde(default)]
    pub summary_input_tokens_total: Option<u64>,
    #[serde(default)]
    pub summary_output_tokens_total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Notify {
    Ingest(EventEnvelope),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub id: Uuid,
    pub payload: Payload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "variant", rename_all = "snake_case")]
pub enum Payload {
    Request(Request),
    Response(Response),
    Notify(Notify),
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    #[test]
    fn payload_request_status_roundtrips() {
        let env = Envelope {
            id: Uuid::new_v4(),
            payload: Payload::Request(Request::Status),
        };
        let j = serde_json::to_string(&env).unwrap();
        let back: Envelope = serde_json::from_str(&j).unwrap();
        assert_eq!(env, back);
    }

    #[test]
    fn search_request_roundtrips() {
        let env = Envelope {
            id: uuid::Uuid::new_v4(),
            payload: Payload::Request(Request::Search(teramind_core::types::SearchRequest {
                query: "stack overflow".into(),
                limit: 5,
            })),
        };
        let j = serde_json::to_string(&env).unwrap();
        let back: Envelope = serde_json::from_str(&j).unwrap();
        assert_eq!(env, back);
    }

    #[test]
    fn search_results_response_roundtrips() {
        let env = Envelope {
            id: uuid::Uuid::new_v4(),
            payload: Payload::Response(Response::SearchResults(
                teramind_core::types::SearchResults {
                    hits: vec![],
                    degraded: false,
                    took_ms: 8,
                },
            )),
        };
        let j = serde_json::to_string(&env).unwrap();
        let back: Envelope = serde_json::from_str(&j).unwrap();
        assert_eq!(env, back);
    }
}
