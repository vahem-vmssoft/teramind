//! Per-session ShareDecision state machine.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use teramind_core::ids::SessionId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareDecision {
    Pending,
    Allowed,
    DeniedKeepLocal,
}

pub struct DecisionCache {
    inner: Mutex<HashMap<SessionId, ShareDecision>>,
}

impl DecisionCache {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { inner: Mutex::new(HashMap::new()) })
    }

    pub fn get(&self, sid: SessionId) -> Option<ShareDecision> {
        self.inner.lock().get(&sid).copied()
    }

    /// Insert if absent; do not overwrite a non-pending state.
    pub fn set_initial(&self, sid: SessionId, d: ShareDecision) {
        let mut m = self.inner.lock();
        m.entry(sid).or_insert(d);
    }

    /// Forcefully update (used when the agent answers).
    /// Returns the previous state.
    pub fn set(&self, sid: SessionId, d: ShareDecision) -> Option<ShareDecision> {
        self.inner.lock().insert(sid, d)
    }

    pub fn evict(&self, sid: SessionId) {
        self.inner.lock().remove(&sid);
    }

    pub fn pending_count(&self) -> usize {
        self.inner.lock().values()
            .filter(|d| **d == ShareDecision::Pending).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn sid() -> SessionId { SessionId(Uuid::new_v4()) }

    #[test]
    fn initial_set_does_not_overwrite() {
        let c = DecisionCache::new();
        let s = sid();
        c.set_initial(s, ShareDecision::Allowed);
        c.set_initial(s, ShareDecision::DeniedKeepLocal);
        assert_eq!(c.get(s), Some(ShareDecision::Allowed));
    }

    #[test]
    fn set_overwrites_returns_prev() {
        let c = DecisionCache::new();
        let s = sid();
        c.set_initial(s, ShareDecision::Pending);
        let prev = c.set(s, ShareDecision::Allowed);
        assert_eq!(prev, Some(ShareDecision::Pending));
        assert_eq!(c.get(s), Some(ShareDecision::Allowed));
    }

    #[test]
    fn pending_count() {
        let c = DecisionCache::new();
        c.set_initial(sid(), ShareDecision::Pending);
        c.set_initial(sid(), ShareDecision::Pending);
        c.set_initial(sid(), ShareDecision::Allowed);
        assert_eq!(c.pending_count(), 2);
    }
}
