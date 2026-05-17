//! Fire-and-forget DB writer for TeamEvents.
//!
//! Every site that calls `bus.send(TeamEvent::...)` also calls `EventLogWriter::log(...)`.
//! The two writes are sequential — bus first (so live subscribers see the event
//! immediately), then a background tokio task does the DB insert. DB failures
//! log a warning; they do NOT block broadcast.

use std::sync::Arc;
use teramind_core::ids::UserId;
use teramind_core::team_event::TeamEvent;
use teramind_db::repos::TeamEventLogRepo;
use tracing::warn;

#[derive(Clone)]
pub struct EventLogWriter {
    repo: TeamEventLogRepo,
}

impl EventLogWriter {
    pub fn new(repo: TeamEventLogRepo) -> Arc<Self> {
        Arc::new(Self { repo })
    }

    pub fn log(self: &Arc<Self>, event: TeamEvent) {
        let me = self.clone();
        tokio::spawn(async move {
            let (kind, user_id, cwd, payload) = match &event {
                TeamEvent::SessionEnded {
                    session_id: _,
                    user_id,
                    cwd,
                    ts: _,
                } => (
                    "session_ended",
                    Some(UserId(*user_id)),
                    Some(cwd.clone()),
                    serde_json::to_value(&event).unwrap_or_default(),
                ),
                TeamEvent::WikiPageReady { user_id, cwd, .. } => (
                    "wiki_page_ready",
                    Some(UserId(*user_id)),
                    Some(cwd.clone()),
                    serde_json::to_value(&event).unwrap_or_default(),
                ),
                TeamEvent::SkillSaved { user_id, .. } => (
                    "skill_saved",
                    Some(UserId(*user_id)),
                    None,
                    serde_json::to_value(&event).unwrap_or_default(),
                ),
            };
            if let Err(e) = me.repo.insert(kind, user_id, cwd, payload).await {
                warn!(error = %e, kind, "event_log insert failed");
            }
        });
    }
}

impl teramindd::services::rpc_dispatch::EventLogger for EventLogWriter {
    fn log(&self, event: teramind_core::team_event::TeamEvent) {
        let repo = self.repo.clone();
        let writer = Arc::new(Self { repo });
        EventLogWriter::log(&writer, event);
    }
}
