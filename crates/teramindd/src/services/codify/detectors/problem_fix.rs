//! Detector B — repeated (error → fix) shapes.

use crate::services::codify::heuristics::{classify_diff, looks_like_error, normalize_error};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use teramind_core::ids::SessionId;
use teramind_db::pool::DbPool;
use teramind_db::repos::SkillObservationRepo;

pub async fn run(
    pool: &DbPool,
    obs: &SkillObservationRepo,
    window: time::Duration,
) -> anyhow::Result<()> {
    let cutoff = time::OffsetDateTime::now_utc() - window;

    // Pull turns + their associated diffs.
    let rows: Vec<(uuid::Uuid, uuid::Uuid, Option<String>, Option<String>)> = sqlx::query_as(
        r#"
        SELECT t.session_id, t.id AS turn_id, t.user_prompt,
               (SELECT string_agg(d.unified_diff, chr(10)) FROM file_diffs d WHERE d.turn_id = t.id) AS diff_agg
        FROM   turns t
        WHERE  t.started_at >= $1
          AND  t.user_prompt IS NOT NULL
        "#)
        .bind(cutoff).fetch_all(pool.pg()).await?;

    let mut sig_to_sessions: HashMap<String, Vec<SessionId>> = HashMap::new();
    let mut sig_to_context: HashMap<String, (String, String)> = HashMap::new();

    for (sid, _tid, prompt_opt, diff_opt) in rows {
        let Some(prompt) = prompt_opt else { continue; };
        let Some(diff) = diff_opt else { continue; };
        if !looks_like_error(&prompt) { continue; }

        let normalized = normalize_error(&prompt);
        let diff_kind = classify_diff(&diff).as_str();

        let mut h = Sha256::new();
        h.update(normalized.as_bytes());
        h.update(b"\x00");
        h.update(diff_kind.as_bytes());
        let sig = hex::encode(&h.finalize()[..8]);

        sig_to_sessions.entry(sig.clone()).or_default().push(SessionId(sid));
        sig_to_context.entry(sig).or_insert((normalized, diff_kind.to_string()));
    }

    for (sig, sessions) in sig_to_sessions {
        let (err, dk) = sig_to_context.get(&sig).cloned().unwrap_or_default();
        let context = serde_json::json!({ "error": err, "diff_kind": dk });
        obs.upsert("problem_fix", &sig, &sessions, context).await?;
    }
    Ok(())
}
