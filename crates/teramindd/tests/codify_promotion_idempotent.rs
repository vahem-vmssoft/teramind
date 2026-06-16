//! codifier §10: restart between INSERT skill and UPDATE candidate must be
//! idempotent — promote_approved_batch uses ON CONFLICT (name) DO UPDATE so a
//! mid-crash skills row is reused and the candidate is correctly marked
//! `promoted`.

use teramind_core::ids::SessionId;
use teramind_db::repos::{SkillCandidateRepo, SkillObservationRepo, SkillRepo};
use teramindd::services::codify::promote::promote_approved_batch;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn promote_is_idempotent_after_crash() -> anyhow::Result<()> {
    let pool = teramind_db::testing::fresh_pool().await?;
    let obs_repo = SkillObservationRepo::new(pool.clone());
    let cand_repo = SkillCandidateRepo::new(pool.clone());
    let skills = SkillRepo::new(pool.clone());

    let sids: Vec<SessionId> = (0..3).map(|_| SessionId(Uuid::new_v4())).collect();
    obs_repo
        .upsert("tool_chain", "sig-x", &sids, serde_json::json!({}))
        .await?;
    let obs = obs_repo
        .find_by_sig("tool_chain", "sig-x")
        .await?
        .expect("obs");

    let cand_id = cand_repo
        .insert(
            obs.id,
            "dup-skill",
            "the description",
            "the body",
            &["/proj".to_string()],
            &sids,
            "test",
            10,
            5,
        )
        .await?;

    // Approve via SQL (same shape as the admin tool).
    sqlx::query("UPDATE skill_candidates SET status='approved', reviewer='admin', reviewed_at=now() WHERE id=$1")
        .bind(cand_id.0)
        .execute(pool.pg())
        .await?;

    // Simulate the mid-crash interleaving: the previous run inserted the skill
    // row but never advanced the candidate to 'promoted'. We insert it directly
    // so promote_approved_batch sees the conflict on (name).
    skills
        .upsert_codified(
            "dup-skill",
            "stale description",
            "stale body",
            &sids.iter().map(|s| s.0).collect::<Vec<_>>(),
            &["/proj".to_string()],
        )
        .await?;
    let (skill_count_before,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM skills WHERE name='dup-skill'")
            .fetch_one(pool.pg())
            .await?;
    assert_eq!(skill_count_before, 1, "pre-crash skills row exists");

    // Re-run the promotion — must complete without error.
    let promoted = promote_approved_batch(&pool, &cand_repo, &skills, 10).await?;
    assert!(promoted >= 1, "promotion must process the approved row");

    // (a) skills row still exists, body updated to candidate's body.
    let (skill_count,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM skills WHERE name='dup-skill'")
            .fetch_one(pool.pg())
            .await?;
    assert_eq!(skill_count, 1, "exactly one skills row for 'dup-skill'");
    let (body,): (String,) = sqlx::query_as("SELECT body FROM skills WHERE name='dup-skill'")
        .fetch_one(pool.pg())
        .await?;
    assert_eq!(body, "the body", "ON CONFLICT DO UPDATE rewrote body");

    // (b) candidate transitions to 'promoted'.
    let (status,): (String,) = sqlx::query_as("SELECT status FROM skill_candidates WHERE id=$1")
        .bind(cand_id.0)
        .fetch_one(pool.pg())
        .await?;
    assert_eq!(status, "promoted");

    Ok(())
}
