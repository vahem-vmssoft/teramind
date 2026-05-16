//! Shared application state passed to every handler.

use crate::config::ServerConfig;
use crate::proof::replay::ReplayCache;
use std::sync::Arc;
use teramind_db::pool::DbPool;
use teramind_db::repos::{DeviceRepo, InviteRepo, UserRepo};
use teramind_core::ids::{DeviceId, UserId};

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
