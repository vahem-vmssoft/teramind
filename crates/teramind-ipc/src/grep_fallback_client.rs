//! Wraps an `RpcTransport` and falls back to grep over local JSONL for read
//! tools (`Search`, `Recall`, `AutoRecall`, `WikiLookup`) when the transport
//! reports a connect failure. Writes (`SaveSkill`) and daemon-control are
//! never fallen back.

use crate::proto::{Request, Response};
use crate::rpc_transport::{RpcError, RpcTransport};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;

pub struct GrepFallback {
    inner: Arc<dyn RpcTransport>,
    jsonl_dir: PathBuf,
}

impl GrepFallback {
    pub fn new(inner: Arc<dyn RpcTransport>, jsonl_dir: PathBuf) -> Self {
        Self { inner, jsonl_dir }
    }
}

#[async_trait]
impl RpcTransport for GrepFallback {
    async fn request(&self, req: Request) -> Result<Response, RpcError> {
        let is_read = matches!(
            req,
            Request::Search(_)
                | Request::Recall(_)
                | Request::AutoRecall(_)
                | Request::WikiLookup { .. }
        );
        match self.inner.request(req.clone()).await {
            Ok(r) => Ok(r),
            Err(e) if !e.is_connect() => Err(e),
            Err(_) if !is_read => {
                Err(RpcError::Connect("server unreachable; write refused".into()))
            }
            Err(_) => fallback(&req, &self.jsonl_dir).await,
        }
    }
}

async fn fallback(req: &Request, jsonl_dir: &std::path::Path) -> Result<Response, RpcError> {
    match req {
        Request::Search(r) => {
            let q = r.query.to_lowercase();
            let mut hits: Vec<teramind_core::types::Hit> = vec![];
            if let Ok(rd) = std::fs::read_dir(jsonl_dir) {
                for entry in rd.flatten() {
                    let p = entry.path();
                    if p.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                        continue;
                    }
                    if let Ok(text) = std::fs::read_to_string(&p) {
                        for line in text.lines().take(10_000) {
                            if line.to_lowercase().contains(&q) {
                                hits.push(make_hit(line));
                                if hits.len() as u32 >= r.limit {
                                    break;
                                }
                            }
                        }
                    }
                    if hits.len() as u32 >= r.limit {
                        break;
                    }
                }
            }
            Ok(Response::SearchResults(teramind_core::types::SearchResults {
                hits,
                degraded: true,
                took_ms: 0,
            }))
        }
        Request::Recall(_) | Request::AutoRecall(_) | Request::WikiLookup { .. } => {
            Ok(Response::SearchResults(teramind_core::types::SearchResults {
                hits: vec![],
                degraded: true,
                took_ms: 0,
            }))
        }
        _ => Err(RpcError::Connect("not fallen back".into())),
    }
}

/// Constructs a `Hit::Turn` from a raw JSONL line as a best-effort snippet.
/// No `GrepLine` variant exists in the Hit enum; `Turn` is the closest match
/// for raw text content. Nil IDs are used since this is a degraded offline hit.
fn make_hit(line: &str) -> teramind_core::types::Hit {
    teramind_core::types::Hit::Turn {
        turn_id: teramind_core::ids::TurnId::nil(),
        session_id: teramind_core::ids::SessionId::nil(),
        ordinal: 0,
        snippet: line.chars().take(512).collect(),
        score: 0.0,
        ts: time::OffsetDateTime::UNIX_EPOCH,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AlwaysConnectFail;

    #[async_trait]
    impl RpcTransport for AlwaysConnectFail {
        async fn request(&self, _: Request) -> Result<Response, RpcError> {
            Err(RpcError::Connect("forced".into()))
        }
    }

    #[tokio::test]
    async fn search_falls_back_to_grep_on_connect_failure() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("2026-05-17.jsonl"),
            "{\"client_event_id\":\"00000000-0000-0000-0000-000000000001\",\"ts\":\"2026-05-17T00:00:00Z\",\"event\":{\"type\":\"user_prompt\",\"session_id\":\"00000000-0000-0000-0000-000000000002\",\"turn_ordinal\":0,\"prompt\":\"hello world\"}}\n",
        )
        .unwrap();
        let g = GrepFallback::new(Arc::new(AlwaysConnectFail), dir.path().to_path_buf());
        let r = g
            .request(Request::Search(teramind_core::types::SearchRequest {
                query: "hello".into(),
                limit: 5,
            }))
            .await
            .unwrap();
        match r {
            Response::SearchResults(s) => {
                assert!(s.degraded);
                assert_eq!(s.hits.len(), 1);
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[tokio::test]
    async fn save_skill_does_not_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        let g = GrepFallback::new(Arc::new(AlwaysConnectFail), dir.path().to_path_buf());
        let r = g
            .request(Request::SaveSkill(teramind_core::types::SaveSkillRequest {
                name: "x".into(),
                description: "y".into(),
                body: "z".into(),
                source_session_ids: vec![],
            }))
            .await;
        assert!(matches!(r, Err(RpcError::Connect(_))));
    }
}
