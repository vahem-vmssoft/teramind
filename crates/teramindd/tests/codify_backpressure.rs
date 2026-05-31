//! codifier §10: when pending candidate count reaches max_pending_candidates,
//! the synthesis loop skips and the open observation is NOT advanced.

use async_trait::async_trait;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use teramind_core::codify::{CodifyDecision, CodifyProvider, CodifyRequest, CodifyResult};
use teramind_core::ids::SessionId;
use teramind_core::redact::Redactor;
use teramind_db::repos::{SkillCandidateRepo, SkillObservationRepo, SkillRepo};
use teramindd::config::CodifyConfig;
use teramindd::services::codifier_worker::{CodifierDeps, CodifierWorker};
use uuid::Uuid;

struct CountingProvider {
    calls: Arc<AtomicU32>,
}
#[async_trait]
impl CodifyProvider for CountingProvider {
    fn name(&self) -> &str {
        "counting"
    }
    async fn codify(&self, _req: CodifyRequest) -> anyhow::Result<CodifyResult> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(CodifyResult {
            decision: CodifyDecision::Skill {
                name: format!("s-{}", Uuid::new_v4()),
                description: "d".into(),
                body: "b".into(),
                applies_to_cwds: vec![],
            },
            input_tokens: 1,
            output_tokens: 1,
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn synthesis_skipped_when_pending_at_cap() -> anyhow::Result<()> {
    let pool = teramind_db::testing::fresh_pool().await?;

    let obs_repo = SkillObservationRepo::new(pool.clone());
    let cand_repo = SkillCandidateRepo::new(pool.clone());
    let skills = SkillRepo::new(pool.clone());

    // Seed an open observation above the min-frequency threshold.
    let sids: Vec<SessionId> = (0..3).map(|_| SessionId(Uuid::new_v4())).collect();
    obs_repo
        .upsert("tool_chain", "open-sig", &sids, serde_json::json!({}))
        .await?;
    let open_obs = obs_repo
        .find_by_sig("tool_chain", "open-sig")
        .await?
        .expect("open obs");

    // Seed a SECOND observation (for the pre-existing pending candidate).
    let other_sids: Vec<SessionId> = (0..3).map(|_| SessionId(Uuid::new_v4())).collect();
    obs_repo
        .upsert("tool_chain", "other-sig", &other_sids, serde_json::json!({}))
        .await?;
    let other_obs = obs_repo
        .find_by_sig("tool_chain", "other-sig")
        .await?
        .expect("other obs");

    // Pre-existing pending candidate fills the back-pressure slot.
    cand_repo
        .insert(
            other_obs.id,
            "preexisting",
            "d",
            "b",
            &[],
            &other_sids,
            "test",
            1,
            1,
        )
        .await?;

    let pending_before = cand_repo.list_pending(10).await?;
    assert_eq!(pending_before.len(), 1);
    let initial_status = open_obs.status.clone();
    assert_eq!(initial_status, "open");

    let mut cfg = CodifyConfig::load_or_default(std::path::Path::new("/nonexistent"));
    cfg.max_pending_candidates = 1; // cap reached
    cfg.min_observation_frequency = 1;

    let calls = Arc::new(AtomicU32::new(0));
    let _worker = CodifierWorker::spawn(CodifierDeps {
        pool: pool.clone(),
        obs: obs_repo.clone(),
        cand: cand_repo.clone(),
        skills: skills.clone(),
        provider: Arc::new(CountingProvider {
            calls: calls.clone(),
        }),
        redactor: Arc::new(Redactor::with_default_rules()),
        cfg,
        run_detectors: false,
        model_label: "mock".into(),
        poll_interval: Duration::from_millis(100),
        cache: None,
    });

    // Allow several ticks.
    tokio::time::sleep(Duration::from_millis(800)).await;

    // (a) provider must NOT have been called: no new candidates produced.
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "back-pressure must skip provider calls"
    );
    let pending_after = cand_repo.list_pending(10).await?;
    assert_eq!(
        pending_after.len(),
        1,
        "no new candidate must appear while back-pressured"
    );

    // (b) the open observation is still 'open' (NOT advanced).
    let still_open = obs_repo
        .find_by_sig("tool_chain", "open-sig")
        .await?
        .expect("obs");
    assert_eq!(still_open.status, "open");

    Ok(())
}
