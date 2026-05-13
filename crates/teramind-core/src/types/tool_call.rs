use crate::ids::{ToolCallId, TurnId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub turn_id: TurnId,
    pub ordinal: i32,
    pub name: String,
    pub input: Value,
    pub output: Option<String>,
    pub is_error: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    pub duration_ms: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tool_call_roundtrips_through_json() {
        let tc = ToolCall {
            id: ToolCallId::new(),
            turn_id: TurnId::new(),
            ordinal: 0,
            name: "Edit".to_string(),
            input: serde_json::json!({"file_path": "/x.rs"}),
            output: Some("ok".to_string()),
            is_error: false,
            started_at: OffsetDateTime::from_unix_timestamp(1_700_000_004).unwrap(),
            duration_ms: Some(42),
        };
        let j = serde_json::to_string(&tc).unwrap();
        let back: ToolCall = serde_json::from_str(&j).unwrap();
        assert_eq!(tc, back);
    }
}
