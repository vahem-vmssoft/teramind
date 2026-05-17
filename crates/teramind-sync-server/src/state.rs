//! Shared application state passed to every handler.

use crate::config::ServerConfig;
use crate::proof::replay::ReplayCache;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use teramind_core::ids::{DeviceId, UserId};
use teramind_db::pool::DbPool;
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo};
use teramind_db::repos::{DeviceRepo, InviteRepo, UserRepo};
use teramindd::services::fs_watcher::WatchRegistry;
use teramindd::services::session_manager::SessionManager;
use teramindd::services::write_tool_ring::WriteToolRing;
use teramindd::RouteDeps;

#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub users: UserRepo,
    pub devices: DeviceRepo,
    pub invites: InviteRepo,
    pub replay: Arc<ReplayCache>,
    pub cfg: Arc<ServerConfig>,
    pub embed_provider: Arc<dyn teramind_core::embed::EmbeddingProvider>,
    pub embed_model: String,
    pub summary_provider: Arc<dyn teramind_core::summarize::SummaryProvider>,
    pub summary_model: String,
    pub bus: tokio::sync::broadcast::Sender<teramind_core::team_event::TeamEvent>,
}

#[derive(Debug, Clone, Copy)]
pub struct AuthContext {
    pub user_id: UserId,
    pub device_id: DeviceId,
}

impl AppState {
    pub fn route_deps(&self) -> RouteDeps {
        let (raw_tx, _raw_rx) =
            tokio::sync::mpsc::unbounded_channel::<teramindd::services::fs_watcher::RawEvent>();
        let gaps = Arc::new(AtomicU64::new(0));
        RouteDeps {
            sessions: SessionManager::new(),
            agents: AgentRepo::new(self.pool.clone()),
            session_repo: SessionRepo::new(self.pool.clone()),
            trace: TraceRepo::new(self.pool.clone()),
            diffs: DiffRepo::new(self.pool.clone()),
            fs_registry: Arc::new(WatchRegistry::new(raw_tx, gaps)),
            write_tool_ring: WriteToolRing::new(64, time::Duration::milliseconds(2000)),
        }
    }

    pub fn rpc_deps(&self) -> teramindd::services::rpc_dispatch::RpcDeps {
        use teramindd::services::search::BlendWeights;
        teramindd::services::rpc_dispatch::RpcDeps {
            pool: self.pool.clone(),
            search_repo: teramind_db::repos::SearchRepo::new(self.pool.clone()),
            wiki_repo: teramind_db::repos::WikiRepo::new(self.pool.clone()),
            embed_provider: self.embed_provider.clone(),
            embed_model: self.embed_model.clone(),
            search_weights: BlendWeights::default(),
            summary_provider: self.summary_provider.clone(),
            summary_model: self.summary_model.clone(),
            jsonl_dir: std::path::PathBuf::new(),
            event_bus: Some(self.bus.clone()),
            skill_obs: teramind_db::repos::SkillObservationRepo::new(self.pool.clone()),
            skill_cand: teramind_db::repos::SkillCandidateRepo::new(self.pool.clone()),
            skill_repo: teramind_db::repos::SkillRepo::new(self.pool.clone()),
            min_observation_frequency: 3,
        }
    }

    pub fn new(pool: DbPool, cfg: ServerConfig) -> Self {
        let replay = ReplayCache::new(
            cfg.auth.proof_replay_cache_size,
            cfg.auth.proof_replay_window_secs as u64,
        );
        let (bus, _rx) =
            tokio::sync::broadcast::channel::<teramind_core::team_event::TeamEvent>(256);
        Self {
            users: UserRepo::new(pool.clone()),
            devices: DeviceRepo::new(pool.clone()),
            invites: InviteRepo::new(pool.clone()),
            pool,
            replay,
            cfg: Arc::new(cfg),
            embed_provider: Arc::new(teramindd::services::embed::NullEmbeddingProvider),
            embed_model: "null".into(),
            summary_provider: Arc::new(teramindd::services::summarize::null::NullSummaryProvider),
            summary_model: "null".into(),
            bus,
        }
    }
}
