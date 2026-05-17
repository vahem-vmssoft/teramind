//! E2E with a mock CodifyProvider that returns a Skill: observation→candidate
//! → SQL-approve → next tick promotes it → skills row exists with source='codified'.

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::codify::{CodifyDecision, CodifyProvider, CodifyRequest, CodifyResult};
use teramind_core::ids::SessionId;
use teramind_core::redact::Redactor;
use teramind_db::repos::{SkillCandidateRepo, SkillObservationRepo, SkillRepo};
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramindd::config::CodifyConfig;
use teramindd::services::codifier_worker::{CodifierDeps, CodifierWorker};
use uuid::Uuid;

struct AlwaysSkill;

#[async_trait]
impl CodifyProvider for AlwaysSkill {
    async fn codify(&self, _: CodifyRequest) -> anyhow::Result<CodifyResult> {
        Ok(CodifyResult {
            decision: CodifyDecision::Skill {
                name: "test-skill".into(),
                description: "desc".into(),
                body: "---\nsource: codified\nseeded_from: 3 sessions\nfirst_observed: 2026-05-17\napplies_to: /proj\n---\n\n# test-skill\n\nbody body body".into(),
                applies_to_cwds: vec!["/proj".into()],
            },
            input_tokens: 100,
            output_tokens: 50,
        })
    }
    fn name(&self) -> &str {
        "always-skill"
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn synthesis_then_approval_promotes() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let obs = SkillObservationRepo::new(pool.clone());
    obs.upsert(
        "tool_chain",
        "sig1",
        &[
            SessionId(Uuid::new_v4()),
            SessionId(Uuid::new_v4()),
            SessionId(Uuid::new_v4()),
        ],
        serde_json::json!({}),
    )
    .await?;

    let cand = SkillCandidateRepo::new(pool.clone());
    let skills = SkillRepo::new(pool.clone());

    let cfg = CodifyConfig::load_or_default(std::path::Path::new("/nonexistent"));
    let _w = CodifierWorker::spawn(CodifierDeps {
        pool: pool.clone(),
        obs: obs.clone(),
        cand: cand.clone(),
        skills: skills.clone(),
        provider: Arc::new(AlwaysSkill),
        redactor: Arc::new(Redactor::with_default_rules()),
        cfg,
        run_detectors: false,
        model_label: "mock".into(),
        poll_interval: Duration::from_millis(100),
        cache: None,
    });

    // Wait for synthesis.
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if !cand.list_pending(10).await?.is_empty() {
            break;
        }
    }
    let pending = cand.list_pending(10).await?;
    assert_eq!(pending.len(), 1);
    let cid = pending[0].id;

    // Approve via SQL.
    sqlx::query("UPDATE skill_candidates SET status='approved', reviewer='admin', reviewed_at=now() WHERE id=$1")
        .bind(cid.0).execute(pool.pg()).await?;

    // Wait for promotion.
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM skills WHERE source='codified'")
            .fetch_one(pool.pg())
            .await?;
        if n == 1 {
            break;
        }
    }
    let (n,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM skills WHERE source='codified' AND name='test-skill'")
            .fetch_one(pool.pg())
            .await?;
    assert_eq!(n, 1, "candidate must be promoted");

    sup.shutdown().await?;
    Ok(())
}
