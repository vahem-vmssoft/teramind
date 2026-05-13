use crate::ids::{SessionId, TurnId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub id: TurnId,
    pub session_id: SessionId,
    pub ordinal: i32,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub ended_at: Option<OffsetDateTime>,
    pub user_prompt: Option<String>,
    pub assistant_text: Option<String>,
    pub thinking: Option<String>,
    pub model: Option<String>,
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn turn_roundtrips_through_json() {
        let t = Turn {
            id: TurnId::new(),
            session_id: SessionId::new(),
            ordinal: 0,
            started_at: OffsetDateTime::from_unix_timestamp(1_700_000_003).unwrap(),
            ended_at: None,
            user_prompt: Some("hello".to_string()),
            assistant_text: None,
            thinking: None,
            model: Some("claude-opus-4-7".to_string()),
            input_tokens: None,
            output_tokens: None,
        };
        let j = serde_json::to_string(&t).unwrap();
        let back: Turn = serde_json::from_str(&j).unwrap();
        assert_eq!(t, back);
    }
}
