use crate::ids::{AgentId, ProjectId, SessionId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEndReason {
    StopHook,
    IdleTimeout,
    Crash,
    Compact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub agent_id: AgentId,
    pub agent_session_id: Option<String>,
    pub cwd: String,
    pub project_id: Option<ProjectId>,
    pub parent_session_id: Option<SessionId>,
    pub git_head: Option<String>,
    pub git_branch: Option<String>,
    pub os: String,
    pub hostname: String,
    pub user_login: String,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub ended_at: Option<OffsetDateTime>,
    pub end_reason: Option<SessionEndReason>,
    #[serde(default)]
    pub metadata: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_roundtrips_through_json() {
        let s = Session {
            id: SessionId::new(),
            agent_id: AgentId::new(),
            agent_session_id: Some("claude-abc".to_string()),
            cwd: "/work".to_string(),
            project_id: None,
            parent_session_id: None,
            git_head: None,
            git_branch: None,
            os: "linux".to_string(),
            hostname: "host".to_string(),
            user_login: "u".to_string(),
            started_at: OffsetDateTime::from_unix_timestamp(1_700_000_002).unwrap(),
            ended_at: None,
            end_reason: None,
            metadata: serde_json::json!({}),
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: Session = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}
