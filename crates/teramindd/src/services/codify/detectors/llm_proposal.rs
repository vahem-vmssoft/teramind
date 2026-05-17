//! Detector C — periodic LLM pass over recent sessions, no rule-based key.

use sha2::{Digest, Sha256};
use teramind_core::codify::{CodifyDecision, CodifyProvider, CodifyRequest};
use teramind_core::ids::SessionId;
use teramind_db::pool::DbPool;
use teramind_db::repos::SkillObservationRepo;

pub async fn run(
    pool: &DbPool,
    obs: &SkillObservationRepo,
    provider: &dyn CodifyProvider,
) -> anyhow::Result<()> {
    // Pick the 5 newest ended sessions.
    let rows: Vec<(uuid::Uuid, String, Option<time::OffsetDateTime>)> = sqlx::query_as(
        r#"SELECT id, cwd, ended_at
           FROM sessions
           WHERE ended_at IS NOT NULL
           ORDER BY ended_at DESC
           LIMIT 5"#)
        .fetch_all(pool.pg()).await?;
    if rows.is_empty() { return Ok(()); }

    // Bundle: wiki excerpts if any, else fall back to last few turns.
    let mut bundle = String::new();
    let mut session_ids: Vec<SessionId> = vec![];
    for (sid, cwd, _) in &rows {
        session_ids.push(SessionId(*sid));
        let wiki: Option<(String,)> = sqlx::query_as(
            r#"SELECT content FROM wiki_pages WHERE session_id = $1 ORDER BY generated_at DESC LIMIT 1"#)
            .bind(sid).fetch_optional(pool.pg()).await?;
        bundle.push_str(&format!("\n## session in {cwd}\n"));
        if let Some((c,)) = wiki {
            bundle.push_str(&c.chars().take(2000).collect::<String>());
        } else {
            bundle.push_str("(no wiki page)\n");
        }
    }

    let cwds: Vec<String> = rows.iter().map(|(_, c, _)| c.clone()).collect();
    let req = CodifyRequest {
        observation_kind: "llm_proposal".into(),
        bundled_context: bundle,
        frequency: rows.len() as u32,
        cwds,
        max_output_tokens: 600,
    };
    let result = provider.codify(req).await?;
    match result.decision {
        CodifyDecision::Skill { name, description, body: _, applies_to_cwds: _ } => {
            // Use the name as the dedup key so re-proposing the same name dedups.
            let mut h = Sha256::new(); h.update(name.as_bytes());
            let sig = hex::encode(&h.finalize()[..8]);
            let ctx = serde_json::json!({
                "proposed_name": name,
                "hint": description,
                "model": provider.name(),
            });
            obs.upsert("llm_proposal", &sig, &session_ids, ctx).await?;
        }
        CodifyDecision::Skip { reason: _ } => {
            // Don't insert anything — the LLM said nothing useful.
        }
    }
    Ok(())
}
