use crate::ids::AgentId;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    pub id: AgentId,
    pub kind: String,
    pub version: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub installed_at: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn agent_roundtrips_through_json() {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let a = Agent {
            id: AgentId::new(),
            kind: "claude_code".to_string(),
            version: Some("0.2.0".to_string()),
            installed_at: now,
        };
        let s = serde_json::to_string(&a).unwrap();
        let back: Agent = serde_json::from_str(&s).unwrap();
        assert_eq!(a, back);
    }
}
