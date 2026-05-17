//! Live-propagation events. Server-side publishers (ingest, summarizer,
//! save_skill) `bus.send(...)` one of these; subscribed daemons receive
//! them via the /v1/events WebSocket.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TeamEvent {
    SessionEnded {
        session_id: Uuid,
        user_id: Uuid,
        cwd: String,
        #[serde(with = "time::serde::rfc3339")]
        ts: time::OffsetDateTime,
    },
    WikiPageReady {
        page_id: Uuid,
        session_id: Uuid,
        user_id: Uuid,
        cwd: String,
        title: String,
        #[serde(with = "time::serde::rfc3339")]
        ts: time::OffsetDateTime,
    },
    SkillSaved {
        skill_id: Uuid,
        user_id: Uuid,
        name: String,
        #[serde(with = "time::serde::rfc3339")]
        ts: time::OffsetDateTime,
    },
}

impl TeamEvent {
    pub fn kind(&self) -> &'static str {
        match self {
            TeamEvent::SessionEnded { .. } => "session_ended",
            TeamEvent::WikiPageReady { .. } => "wiki_page_ready",
            TeamEvent::SkillSaved { .. } => "skill_saved",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let evt = TeamEvent::SessionEnded {
            session_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            cwd: "/repo".into(),
            ts: time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        };
        let j = serde_json::to_string(&evt).unwrap();
        let parsed: TeamEvent = serde_json::from_str(&j).unwrap();
        match parsed {
            TeamEvent::SessionEnded { cwd, .. } => assert_eq!(cwd, "/repo"),
            other => panic!("unexpected: {other:?}"),
        }
    }
}
