use crate::services::fs_watcher::WatchRegistry;
use crate::services::session_manager::SessionManager;
use std::sync::Arc;
use std::time::Duration;
use teramind_db::repos::SessionRepo;
use time::OffsetDateTime;
use tracing::{info, warn};

pub struct IdleSessionSweeper {
    handle: tokio::task::JoinHandle<()>,
}

impl IdleSessionSweeper {
    pub fn spawn(
        sessions: SessionManager,
        session_repo: SessionRepo,
        fs_registry: Arc<WatchRegistry>,
        idle_timeout: Duration,
    ) -> Self {
        let poll_interval = Duration::from_secs(15 * 60);
        let handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(poll_interval).await;
                sweep_once(&sessions, &session_repo, &fs_registry, idle_timeout).await;
            }
        });
        Self { handle }
    }

    pub fn abort(&self) {
        self.handle.abort();
    }
}

pub async fn sweep_once(
    sessions: &SessionManager,
    session_repo: &SessionRepo,
    fs_registry: &Arc<WatchRegistry>,
    idle_timeout: Duration,
) {
    let cutoff = OffsetDateTime::now_utc() - time::Duration::seconds(idle_timeout.as_secs() as i64);
    for s in sessions.idle_since(cutoff).await {
        match session_repo
            .end(s.session_id, OffsetDateTime::now_utc(), "idle_timeout")
            .await
        {
            Ok(_) => {
                sessions.end(s.session_id).await;
                fs_registry
                    .unregister(std::path::Path::new(&s.cwd), s.session_id)
                    .await;
                info!(session_id = %s.session_id, "idle_session_sweeper closed session");
            }
            Err(e) => {
                warn!(error = %e, session_id = %s.session_id, "idle_session_sweeper failed to close session")
            }
        }
    }
}
