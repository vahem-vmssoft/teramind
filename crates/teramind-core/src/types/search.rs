use crate::ids::{SessionId, SkillId};
use crate::types::Hit;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default = "SearchRequest::default_limit")]
    pub limit: u32,
}

impl SearchRequest {
    fn default_limit() -> u32 { 10 }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RecallRequest {
    pub cwd: Option<String>,
    #[serde(default)]
    pub file_paths: Vec<String>,
    #[serde(default)]
    pub symbols: Vec<String>,
    #[serde(default)]
    pub stack_traces: Vec<String>,
    #[serde(default = "RecallRequest::default_limit")]
    pub limit: u32,
}

impl RecallRequest {
    fn default_limit() -> u32 { 10 }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoRecallRequest {
    pub cwd: String,
    #[serde(default = "AutoRecallRequest::default_limit")]
    pub limit: u32,
}

impl AutoRecallRequest {
    fn default_limit() -> u32 { 5 }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveSkillRequest {
    pub name: String,
    pub description: String,
    pub body: String,
    #[serde(default)]
    pub source_session_ids: Vec<SessionId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResults {
    pub hits: Vec<Hit>,
    #[serde(default)]
    pub degraded: bool,
    pub took_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillRef {
    pub id: SkillId,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_request_default_limit_when_missing() {
        let r: SearchRequest = serde_json::from_str(r#"{"query":"x"}"#).unwrap();
        assert_eq!(r.limit, 10);
    }

    #[test]
    fn search_results_roundtrips() {
        let r = SearchResults { hits: vec![], degraded: false, took_ms: 42 };
        let j = serde_json::to_string(&r).unwrap();
        assert_eq!(r, serde_json::from_str(&j).unwrap());
    }

    #[test]
    fn auto_recall_request_default_limit() {
        let r: AutoRecallRequest = serde_json::from_str(r#"{"cwd":"/w"}"#).unwrap();
        assert_eq!(r.limit, 5);
        assert_eq!(r.cwd, "/w");
    }
}
