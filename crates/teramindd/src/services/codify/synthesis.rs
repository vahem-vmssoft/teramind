//! Bundles context for an observation and calls the CodifyProvider.

use std::sync::Arc;
use teramind_core::codify::{CodifyDecision, CodifyProvider, CodifyRequest};
use teramind_core::redact::Redactor;
use teramind_db::pool::DbPool;
use teramind_db::repos::{Candidate, Observation, SkillCandidateRepo, SkillObservationRepo};

pub struct SynthesisDeps {
    pub pool: DbPool,
    pub obs: SkillObservationRepo,
    pub cand: SkillCandidateRepo,
    pub provider: Arc<dyn CodifyProvider>,
    pub redactor: Arc<Redactor>,
    pub input_char_budget: usize,
    pub output_token_budget: u32,
    pub model_label: String,
}

pub async fn synthesize_one(
    deps: &SynthesisDeps,
    observation: Observation,
) -> anyhow::Result<Option<Candidate>> {
    let mut bundle = bundle_context(&deps.pool, &observation, deps.input_char_budget).await?;
    bundle = deps.redactor.apply(&bundle);

    let cwds = collect_cwds(&deps.pool, &observation.session_ids).await?;

    let req = CodifyRequest {
        observation_kind: observation.kind.clone(),
        bundled_context: bundle,
        frequency: observation.frequency as u32,
        cwds,
        max_output_tokens: deps.output_token_budget,
    };
    let result = deps.provider.codify(req).await?;

    match result.decision {
        CodifyDecision::Skip { reason } => {
            deps.obs.mark_skipped(observation.id, &reason).await?;
            Ok(None)
        }
        CodifyDecision::Skill {
            name,
            description,
            body,
            applies_to_cwds,
        } => {
            let session_ids: Vec<teramind_core::ids::SessionId> = observation
                .session_ids
                .iter()
                .copied()
                .map(teramind_core::ids::SessionId)
                .collect();
            let cand_id = deps
                .cand
                .insert(
                    observation.id,
                    &name,
                    &description,
                    &body,
                    &applies_to_cwds,
                    &session_ids,
                    &deps.model_label,
                    result.input_tokens as i32,
                    result.output_tokens as i32,
                )
                .await?;
            // Supersede any older pending candidates for the same observation.
            let _ = deps.cand.supersede_prior(observation.id, cand_id).await;
            deps.obs.mark_synthesized(observation.id).await?;
            Ok(Some(Candidate {
                id: cand_id,
                observation_id: observation.id,
                name,
                description,
                body,
                applies_to_cwds,
                source_session_ids: observation.session_ids.clone(),
                model: deps.model_label.clone(),
                input_tokens: result.input_tokens as i32,
                output_tokens: result.output_tokens as i32,
                generated_at: time::OffsetDateTime::now_utc(),
                status: "pending".into(),
                reviewer: None,
                reviewed_at: None,
            }))
        }
    }
}

async fn bundle_context(pool: &DbPool, obs: &Observation, budget: usize) -> anyhow::Result<String> {
    let mut out = format!(
        "Observation kind: {}\nSignature: {}\nFrequency: {}\nContext: {}\n\n",
        obs.kind, obs.signature, obs.frequency, obs.context_blob
    );
    for sid in obs.session_ids.iter().take(5) {
        // Wiki excerpt first (most signal-dense).
        if let Some((content,)) = sqlx::query_as::<_, (String,)>(
            r#"SELECT content FROM wiki_pages WHERE session_id = $1 ORDER BY generated_at DESC LIMIT 1"#)
            .bind(sid).fetch_optional(pool.pg()).await?
        {
            out.push_str(&format!("\n## session {sid} — wiki\n"));
            out.push_str(&content.chars().take(3000).collect::<String>());
        } else {
            // Representative turns (up to 3) when no wiki exists.
            let turns: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
                r#"SELECT user_prompt, assistant_text
                   FROM turns WHERE session_id = $1
                   ORDER BY ordinal LIMIT 3"#)
                .bind(sid).fetch_all(pool.pg()).await?;
            out.push_str(&format!("\n## session {sid} — turns\n"));
            for (p, a) in turns {
                if let Some(p) = p { out.push_str(&format!("> {p}\n")); }
                if let Some(a) = a { out.push_str(&format!("{a}\n")); }
            }
        }
        if out.len() > budget {
            const MARKER: &str = "\n…[truncated]";
            let cap = budget.saturating_sub(MARKER.len());
            let mut end = cap.min(out.len());
            while end > 0 && !out.is_char_boundary(end) {
                end -= 1;
            }
            out.truncate(end);
            out.push_str(MARKER);
            break;
        }
    }
    Ok(out)
}

async fn collect_cwds(pool: &DbPool, session_ids: &[uuid::Uuid]) -> anyhow::Result<Vec<String>> {
    let rows: Vec<(String,)> =
        sqlx::query_as(r#"SELECT DISTINCT cwd FROM sessions WHERE id = ANY($1)"#)
            .bind(session_ids)
            .fetch_all(pool.pg())
            .await?;
    Ok(rows.into_iter().map(|(c,)| c).collect())
}
