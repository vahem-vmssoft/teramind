use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use teramind_core::ids::{SessionId, TurnId};
use time::OffsetDateTime;

#[derive(Debug, Clone)]
pub struct ActiveSession {
    pub session_id: SessionId,
    pub cwd: String,
    pub agent_kind: String,
    pub started_at: OffsetDateTime,
    pub last_activity: OffsetDateTime,
    pub last_turn_id: Option<TurnId>,
}

#[derive(Clone, Default)]
pub struct SessionManager {
    inner: Arc<RwLock<HashMap<SessionId, ActiveSession>>>,
}

impl SessionManager {
    pub fn new() -> Self { Self::default() }

    pub async fn start(&self, s: ActiveSession) {
        self.inner.write().await.insert(s.session_id, s);
    }
    pub async fn touch(&self, id: SessionId, at: OffsetDateTime, turn_id: Option<TurnId>) {
        if let Some(s) = self.inner.write().await.get_mut(&id) {
            s.last_activity = at;
            if turn_id.is_some() { s.last_turn_id = turn_id; }
        }
    }
    pub async fn end(&self, id: SessionId) -> Option<ActiveSession> {
        self.inner.write().await.remove(&id)
    }
    pub async fn get(&self, id: SessionId) -> Option<ActiveSession> {
        self.inner.read().await.get(&id).cloned()
    }
    pub async fn idle_since(&self, cutoff: OffsetDateTime) -> Vec<ActiveSession> {
        self.inner.read().await.values().filter(|s| s.last_activity < cutoff).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::OffsetDateTime;

    #[tokio::test]
    async fn manager_lifecycle() {
        let m = SessionManager::new();
        let sid = SessionId::new();
        let now = OffsetDateTime::now_utc();
        m.start(ActiveSession {
            session_id: sid, cwd: "/w".into(), agent_kind: "claude_code".into(),
            started_at: now, last_activity: now, last_turn_id: None,
        }).await;
        assert!(m.get(sid).await.is_some());
        m.touch(sid, now + time::Duration::seconds(5), None).await;
        let removed = m.end(sid).await;
        assert!(removed.is_some());
        assert!(m.get(sid).await.is_none());
    }

    #[tokio::test]
    async fn idle_since_filters() {
        let m = SessionManager::new();
        let sid = SessionId::new();
        let t0 = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        m.start(ActiveSession { session_id: sid, cwd: "/".into(), agent_kind: "c".into(),
                                started_at: t0, last_activity: t0, last_turn_id: None }).await;
        let stale = m.idle_since(t0 + time::Duration::seconds(1)).await;
        assert_eq!(stale.len(), 1);
        let fresh = m.idle_since(t0 - time::Duration::seconds(1)).await;
        assert_eq!(fresh.len(), 0);
    }
}
