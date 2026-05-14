//! Bounded ring of recent write-tool PostToolUse completions.
//! Used by the FS watcher to decide whether a file change should be
//! attributed to an agent turn (within `window`) or to the human user.

use std::collections::VecDeque;
use std::sync::Arc;
use teramind_core::ids::{SessionId, TurnId};
use time::OffsetDateTime;
use tokio::sync::Mutex;

pub const WRITE_TOOLS: &[&str] = &["Edit", "Write", "MultiEdit", "NotebookEdit"];

#[derive(Debug, Clone)]
pub struct WriteCompletion {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub tool_name: String,
    pub at: OffsetDateTime,
}

#[derive(Clone)]
pub struct WriteToolRing {
    inner: Arc<Mutex<VecDeque<WriteCompletion>>>,
    capacity: usize,
    window: time::Duration,
}

impl WriteToolRing {
    pub fn new(capacity: usize, window: time::Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
            window,
        }
    }

    pub async fn push(&self, w: WriteCompletion) {
        let mut d = self.inner.lock().await;
        if d.len() == self.capacity {
            d.pop_front();
        }
        d.push_back(w);
    }

    /// Find the most recent write completion for `session_id` no older than
    /// `now - window`. Returns the matching record or `None`.
    pub async fn most_recent_for(
        &self,
        session_id: SessionId,
        now: OffsetDateTime,
    ) -> Option<WriteCompletion> {
        let d = self.inner.lock().await;
        let cutoff = now - self.window;
        d.iter()
            .rev()
            .find(|w| w.session_id == session_id && w.at >= cutoff)
            .cloned()
    }
}

pub fn is_write_tool(name: &str) -> bool {
    WRITE_TOOLS.iter().any(|w| *w == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(secs: i64) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(secs).unwrap()
    }

    #[tokio::test]
    async fn push_and_find_inside_window() {
        let ring = WriteToolRing::new(8, time::Duration::seconds(5));
        let sid = SessionId::new();
        let tid = TurnId::new();
        ring.push(WriteCompletion {
            session_id: sid, turn_id: tid, tool_name: "Edit".into(), at: t(100),
        }).await;
        let got = ring.most_recent_for(sid, t(103)).await;
        assert!(got.is_some());
        assert_eq!(got.unwrap().turn_id, tid);
    }

    #[tokio::test]
    async fn outside_window_returns_none() {
        let ring = WriteToolRing::new(8, time::Duration::seconds(5));
        let sid = SessionId::new();
        ring.push(WriteCompletion {
            session_id: sid, turn_id: TurnId::new(), tool_name: "Edit".into(), at: t(100),
        }).await;
        assert!(ring.most_recent_for(sid, t(200)).await.is_none());
    }

    #[tokio::test]
    async fn capacity_evicts_oldest() {
        let ring = WriteToolRing::new(2, time::Duration::seconds(60));
        let sid = SessionId::new();
        for i in 0..4 {
            ring.push(WriteCompletion {
                session_id: sid,
                turn_id: TurnId::new(),
                tool_name: "Edit".into(),
                at: t(100 + i),
            }).await;
        }
        // Only the newest two survive; oldest at t(100) should be gone.
        let got = ring.most_recent_for(sid, t(105)).await.unwrap();
        assert_eq!(got.at, t(103));
    }

    #[test]
    fn is_write_tool_matches_documented_names() {
        assert!(is_write_tool("Edit"));
        assert!(is_write_tool("Write"));
        assert!(is_write_tool("MultiEdit"));
        assert!(is_write_tool("NotebookEdit"));
        assert!(!is_write_tool("Read"));
        assert!(!is_write_tool("Bash"));
    }
}
