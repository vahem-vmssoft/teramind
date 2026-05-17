//! Daemon-side wrappers around teramind_core::team_share.
//! Re-exports the core helpers + provides `DaemonTeamShareSetter` which
//! satisfies the `TeamShareSetter` trait used by the IPC handler.

pub use teramind_core::team_share::{find_marker, write_marker_at_cwd, ShareMarker};

use crate::services::decision_cache::{DecisionCache, ShareDecision};
use crate::services::ipc_server::TeamShareSetter;
use async_trait::async_trait;
use std::sync::Arc;
use teramind_core::ids::SessionId;

pub struct DaemonTeamShareSetter {
    pub cache: Arc<DecisionCache>,
    pub user_email: String,
}

#[async_trait]
impl TeamShareSetter for DaemonTeamShareSetter {
    async fn write_and_signal(
        &self,
        cwd: &std::path::Path,
        session_id: Option<SessionId>,
        share: bool,
        _set_by: &str,
    ) -> anyhow::Result<()> {
        let marker = ShareMarker {
            share,
            set_by: self.user_email.clone(),
            set_at: time::OffsetDateTime::now_utc(),
        };
        write_marker_at_cwd(cwd, &marker)?;
        if let Some(sid) = session_id {
            let new = if share {
                ShareDecision::Allowed
            } else {
                ShareDecision::DeniedKeepLocal
            };
            self.cache.set(sid, new);
        }
        Ok(())
    }
}
