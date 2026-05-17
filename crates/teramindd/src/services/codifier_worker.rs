//! Three-loop orchestrator: synthesize + promote (poll) + detectors (long cycle).

use crate::config::CodifyConfig;
use crate::services::codify::detectors::{llm_proposal, problem_fix, tool_chain};
use crate::services::codify::promote::promote_approved_batch;
use crate::services::codify::synthesis::{synthesize_one, SynthesisDeps};
use crate::services::decision_cache::DecisionCache;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::codify::CodifyProvider;
use teramind_core::redact::Redactor;
use teramind_db::pool::DbPool;
use teramind_db::repos::{SkillCandidateRepo, SkillObservationRepo, SkillRepo};
use tracing::{info, warn};

#[derive(Default)]
pub struct CodifierStats {
    pub observations_total: AtomicU64,
    pub candidates_total: AtomicU64,
    pub promotions_total: AtomicU64,
    pub skips_total: AtomicU64,
}

pub struct CodifierDeps {
    pub pool: DbPool,
    pub obs: SkillObservationRepo,
    pub cand: SkillCandidateRepo,
    pub skills: SkillRepo,
    pub provider: Arc<dyn CodifyProvider>,
    pub redactor: Arc<Redactor>,
    pub cfg: CodifyConfig,
    /// Test escape hatch: when false, the detector loop never runs.
    pub run_detectors: bool,
    pub model_label: String,
    pub poll_interval: Duration,
    /// Optional privacy filter: sessions with `DeniedKeepLocal` are excluded
    /// from all detector observation seed sets.
    pub cache: Option<Arc<DecisionCache>>,
}

pub struct CodifierWorker {
    pub stats: Arc<CodifierStats>,
    _h_synth: tokio::task::JoinHandle<()>,
    _h_detect: Option<tokio::task::JoinHandle<()>>,
}

impl CodifierWorker {
    pub fn spawn(deps: CodifierDeps) -> Self {
        let stats = Arc::new(CodifierStats::default());

        let synth_deps = deps.clone_for_synth(stats.clone());
        let h_synth = tokio::spawn(async move { synthesis_promote_loop(synth_deps).await });

        let h_detect = if deps.run_detectors {
            let dep = deps.clone_for_detectors(stats.clone());
            Some(tokio::spawn(async move { detector_loop(dep).await }))
        } else {
            None
        };

        Self {
            stats,
            _h_synth: h_synth,
            _h_detect: h_detect,
        }
    }
}

impl CodifierDeps {
    fn clone_for_synth(&self, _stats: Arc<CodifierStats>) -> SynthAndPromoteLoop {
        SynthAndPromoteLoop {
            pool: self.pool.clone(),
            obs: self.obs.clone(),
            cand: self.cand.clone(),
            skills: self.skills.clone(),
            provider: self.provider.clone(),
            redactor: self.redactor.clone(),
            cfg: self.cfg.clone(),
            model_label: self.model_label.clone(),
            poll_interval: self.poll_interval,
        }
    }
    fn clone_for_detectors(&self, _stats: Arc<CodifierStats>) -> DetectorLoop {
        DetectorLoop {
            pool: self.pool.clone(),
            obs: self.obs.clone(),
            provider: self.provider.clone(),
            cfg: self.cfg.clone(),
            cache: self.cache.clone(),
        }
    }
}

struct SynthAndPromoteLoop {
    pool: DbPool,
    obs: SkillObservationRepo,
    cand: SkillCandidateRepo,
    skills: SkillRepo,
    provider: Arc<dyn CodifyProvider>,
    redactor: Arc<Redactor>,
    cfg: CodifyConfig,
    model_label: String,
    poll_interval: Duration,
}

async fn synthesis_promote_loop(d: SynthAndPromoteLoop) {
    loop {
        // 1. Promote any approved candidates.
        if let Err(e) = promote_approved_batch(&d.pool, &d.cand, &d.skills, 10).await {
            warn!(error = %e, "promote_approved_batch error");
        }

        // 2. Back-pressure: skip synthesis when too many pending.
        let pending_n = d
            .cand
            .list_pending(d.cfg.max_pending_candidates)
            .await
            .map(|v| v.len() as i64)
            .unwrap_or(0);
        if pending_n >= d.cfg.max_pending_candidates {
            tokio::time::sleep(d.poll_interval).await;
            continue;
        }

        // 3. Pick one open observation above threshold.
        let open = d
            .obs
            .list_open(d.cfg.min_observation_frequency, 1)
            .await
            .ok()
            .unwrap_or_default();
        if let Some(o) = open.into_iter().next() {
            let deps = SynthesisDeps {
                pool: d.pool.clone(),
                obs: d.obs.clone(),
                cand: d.cand.clone(),
                provider: d.provider.clone(),
                redactor: d.redactor.clone(),
                input_char_budget: d.cfg.input_char_budget,
                output_token_budget: d.cfg.output_token_budget,
                model_label: d.model_label.clone(),
            };
            match synthesize_one(&deps, o).await {
                Ok(Some(_)) => info!("synthesized candidate"),
                Ok(None) => info!("observation skipped"),
                Err(e) => warn!(error = %e, "synthesis error"),
            }
        }

        tokio::time::sleep(d.poll_interval).await;
    }
}

struct DetectorLoop {
    pool: DbPool,
    obs: SkillObservationRepo,
    provider: Arc<dyn CodifyProvider>,
    cfg: CodifyConfig,
    cache: Option<Arc<DecisionCache>>,
}

async fn detector_loop(d: DetectorLoop) {
    loop {
        if d.cfg.detectors.tool_chain {
            if let Err(e) =
                tool_chain::run(&d.pool, &d.obs, time::Duration::days(30), d.cache.clone()).await
            {
                warn!(error = %e, "tool_chain detector error");
            }
        }
        if d.cfg.detectors.problem_fix {
            if let Err(e) =
                problem_fix::run(&d.pool, &d.obs, time::Duration::days(30), d.cache.clone()).await
            {
                warn!(error = %e, "problem_fix detector error");
            }
        }
        if d.cfg.detectors.llm_proposal {
            if let Err(e) =
                llm_proposal::run(&d.pool, &d.obs, d.provider.as_ref(), d.cache.clone()).await
            {
                warn!(error = %e, "llm_proposal detector error");
            }
        }
        tokio::time::sleep(Duration::from_secs(d.cfg.autonomous_cycle_secs)).await;
    }
}
