//! Transactional promotion of approved candidates into the live skills table.

use teramind_db::pool::DbPool;
use teramind_db::repos::{SkillCandidateRepo, SkillRepo};
use tracing::{info, warn};

pub async fn promote_approved_batch(
    pool: &DbPool,
    candidates: &SkillCandidateRepo,
    skills: &SkillRepo,
    limit: i64,
) -> anyhow::Result<u64> {
    let approved = candidates.list_approved(limit).await?;
    let mut count = 0u64;
    for c in approved {
        let tx = match pool.pg().begin().await {
            Ok(tx) => tx,
            Err(e) => {
                warn!(error = %e, candidate = %c.id.0, "begin transaction failed; skipping");
                continue;
            }
        };

        let skill_res = skills.upsert_codified(
            &c.name, &c.description, &c.body,
            &c.source_session_ids,
            &c.applies_to_cwds,
        ).await;
        match skill_res {
            Ok(_) => {
                if let Err(e) = candidates.mark_promoted(c.id).await {
                    warn!(error = %e, candidate = %c.id.0, "mark_promoted failed; rolling back");
                    let _ = tx.rollback().await;
                    continue;
                }
                if let Err(e) = tx.commit().await {
                    warn!(error = %e, candidate = %c.id.0, "commit failed");
                    continue;
                }
                info!(name = %c.name, "candidate promoted to skill");
                count += 1;
            }
            Err(e) => {
                warn!(error = %e, candidate = %c.id.0, "promotion upsert failed");
                let _ = tx.rollback().await;
            }
        }
    }
    Ok(count)
}
