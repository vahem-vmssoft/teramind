//! Shared application state passed to every handler.

use crate::config::ServerConfig;
use crate::proof::replay::ReplayCache;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use teramind_db::pool::DbPool;
use teramind_db::repos::{DeviceRepo, InviteRepo, UserRepo};
use teramind_core::ids::{DeviceId, UserId};
use teramindd::RouteDeps;
use teramindd::services::session_manager::SessionManager;
use teramindd::services::write_tool_ring::WriteToolRing;
use teramindd::services::fs_watcher::WatchRegistry;
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo};

#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub users: UserRepo,
    pub devices: DeviceRepo,
    pub invites: InviteRepo,
    pub replay: Arc<ReplayCache>,
    pub cfg: Arc<ServerConfig>,
}

#[derive(Debug, Clone, Copy)]
pub struct AuthContext {
    pub user_id: UserId,
    pub device_id: DeviceId,
}

impl AppState {
    pub fn route_deps(&self) -> RouteDeps {
        let (raw_tx, _raw_rx) = tokio::sync::mpsc::unbounded_channel::<teramindd::services::fs_watcher::RawEvent>();
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

    pub fn new(pool: DbPool, cfg: ServerConfig) -> Self {
        let replay = ReplayCache::new(
            cfg.auth.proof_replay_cache_size,
            cfg.auth.proof_replay_window_secs as u64,
        );
        Self {
            users: UserRepo::new(pool.clone()),
            devices: DeviceRepo::new(pool.clone()),
            invites: InviteRepo::new(pool.clone()),
            pool,
            replay,
            cfg: Arc::new(cfg),
        }
    }
}
