use serde::{Deserialize, Serialize};
use teramind_core::types::ingest_event::EventEnvelope;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Request {
    Status,
    Ping,
    Shutdown,
    // Plan C/D add Search { ... }, Recall { ... }, SaveSkill { ... }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    Ok,
    Pong,
    Status(StatusReport),
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusReport {
    pub uptime_seconds: u64,
    pub pg_connected: bool,
    pub ingest_queue_depth: u32,
    pub ingest_drops_total: u64,
    pub last_storage_pg_bytes: i64,
    pub last_storage_jsonl_bytes: i64,
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
#[serde(tag = "kind", rename_all = "snake_case")]
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
        let env = Envelope { id: Uuid::new_v4(), payload: Payload::Request(Request::Status) };
        let j = serde_json::to_string(&env).unwrap();
        let back: Envelope = serde_json::from_str(&j).unwrap();
        assert_eq!(env, back);
    }
}
