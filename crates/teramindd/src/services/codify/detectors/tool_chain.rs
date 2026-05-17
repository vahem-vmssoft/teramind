//! Detector A — repeated tool-call sequences.

use crate::services::codify::detectors::is_denied;
use crate::services::codify::heuristics::{bash_head_verb, file_kind};
use crate::services::decision_cache::DecisionCache;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use teramind_core::ids::SessionId;
use teramind_db::pool::DbPool;
use teramind_db::repos::SkillObservationRepo;

#[derive(Debug, Clone)]
struct CallRow {
    #[allow(dead_code)]
    session_id: uuid::Uuid,
    tool_name: String,
    input: serde_json::Value,
    #[allow(dead_code)]
    started_at: time::OffsetDateTime,
}

pub async fn run(
    pool: &DbPool,
    obs: &SkillObservationRepo,
    window: time::Duration,
    cache: Option<Arc<DecisionCache>>,
) -> anyhow::Result<()> {
    let cutoff = time::OffsetDateTime::now_utc() - window;

    let rows: Vec<(uuid::Uuid, String, serde_json::Value, time::OffsetDateTime)> = sqlx::query_as(
        r#"SELECT t.session_id, tc.name, tc.input, tc.started_at
           FROM tool_calls tc
           JOIN turns t ON t.id = tc.turn_id
           JOIN sessions s ON s.id = t.session_id
           WHERE tc.started_at >= $1
           ORDER BY t.session_id, tc.ordinal"#,
    )
    .bind(cutoff)
    .fetch_all(pool.pg())
    .await?;

    let mut per_session: HashMap<uuid::Uuid, Vec<CallRow>> = HashMap::new();
    for (sid, name, input, ts) in rows {
        if is_denied(&cache, sid) {
            continue;
        }
        per_session.entry(sid).or_default().push(CallRow {
            session_id: sid,
            tool_name: name,
            input,
            started_at: ts,
        });
    }

    // Per session, build a signature tuple list.
    let mut sig_to_sessions: HashMap<String, Vec<SessionId>> = HashMap::new();
    let mut sig_to_chain: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for (sid, calls) in per_session {
        let tuples: Vec<(String, String)> = calls
            .iter()
            .map(|c| {
                let head = match c.tool_name.as_str() {
                    "Bash" => bash_head_verb(
                        c.input
                            .get("command")
                            .and_then(|v| v.as_str())
                            .unwrap_or(""),
                    )
                    .to_string(),
                    "Edit" | "Write" | "Read" => file_kind(
                        c.input
                            .get("file_path")
                            .and_then(|v| v.as_str())
                            .unwrap_or(""),
                    ),
                    _ => String::new(),
                };
                (c.tool_name.clone(), head)
            })
            .collect();
        if tuples.len() < 2 {
            continue;
        }
        let mut hasher = Sha256::new();
        for (t, h) in &tuples {
            hasher.update(t.as_bytes());
            hasher.update(b"\x00");
            hasher.update(h.as_bytes());
            hasher.update(b"\x01");
        }
        let sig = hex::encode(&hasher.finalize()[..8]);
        sig_to_sessions
            .entry(sig.clone())
            .or_default()
            .push(SessionId(sid));
        sig_to_chain.entry(sig).or_insert(tuples);
    }

    for (sig, sessions) in sig_to_sessions {
        if sessions.is_empty() {
            continue;
        }
        let chain = sig_to_chain.get(&sig).cloned().unwrap_or_default();
        let context = serde_json::json!({
            "head_chain": chain.iter().map(|(t, h)| format!("{t}({h})")).collect::<Vec<_>>(),
        });
        obs.upsert("tool_chain", &sig, &sessions, context).await?;
    }
    Ok(())
}
