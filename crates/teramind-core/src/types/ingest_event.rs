use crate::ids::{ClientEventId, SessionId, ToolCallId, TurnId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub client_event_id: ClientEventId,
    #[serde(with = "time::serde::rfc3339")]
    pub ts: OffsetDateTime,
    pub event: IngestEvent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IngestEvent {
    SessionStart {
        session_id: SessionId,
        agent_session_id: Option<String>,
        agent_kind: String,
        cwd: String,
        os: String,
        hostname: String,
        user_login: String,
        git_head: Option<String>,
        git_branch: Option<String>,
    },
    UserPrompt {
        session_id: SessionId,
        turn_ordinal: i32,
        prompt: String,
        #[serde(default)]
        turn_id: Option<TurnId>,
    },
    ToolCallStart {
        turn_id: TurnId,
        #[serde(default)]
        tool_call_id: Option<ToolCallId>,
        ordinal: i32,
        name: String,
        input: Value,
    },
    ToolCallEnd {
        tool_call_id: ToolCallId,
        output: String,
        is_error: bool,
        duration_ms: i32,
        #[serde(default)]
        session_id: Option<SessionId>,
        #[serde(default)]
        turn_id: Option<TurnId>,
        #[serde(default)]
        tool_name: Option<String>,
    },
    AssistantTurn {
        turn_id: TurnId,
        assistant_text: String,
        thinking: Option<String>,
        model: Option<String>,
        input_tokens: Option<i32>,
        output_tokens: Option<i32>,
    },
    SessionEnd {
        session_id: SessionId,
        reason: String,
    },
    PreCompact {
        session_id: SessionId,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn envelope_roundtrips() {
        let env = EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::from_unix_timestamp(1_700_000_010).unwrap(),
            event: IngestEvent::UserPrompt {
                session_id: SessionId::new(),
                turn_ordinal: 0,
                prompt: "hi".into(),
                turn_id: None,
            },
        };
        let j = serde_json::to_string(&env).unwrap();
        let back: EventEnvelope = serde_json::from_str(&j).unwrap();
        assert_eq!(env, back);
    }

    #[test]
    fn tool_call_end_carries_optional_metadata() {
        let env = EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::from_unix_timestamp(1_700_000_020).unwrap(),
            event: IngestEvent::ToolCallEnd {
                tool_call_id: ToolCallId::new(),
                output: "ok".into(),
                is_error: false,
                duration_ms: 10,
                session_id: Some(SessionId::new()),
                turn_id: Some(TurnId::new()),
                tool_name: Some("Edit".into()),
            },
        };
        let j = serde_json::to_string(&env).unwrap();
        let back: EventEnvelope = serde_json::from_str(&j).unwrap();
        assert_eq!(env, back);
    }

    #[test]
    fn tool_call_end_back_compat_no_metadata() {
        // Older envelopes without the new fields must still deserialize.
        let j = r#"{"client_event_id":"00000000-0000-0000-0000-000000000001","ts":"2026-05-14T00:00:00Z","event":{"type":"tool_call_end","tool_call_id":"00000000-0000-0000-0000-000000000002","output":"x","is_error":false,"duration_ms":1}}"#;
        let env: EventEnvelope = serde_json::from_str(j).unwrap();
        match env.event {
            IngestEvent::ToolCallEnd { session_id, turn_id, tool_name, .. } => {
                assert!(session_id.is_none());
                assert!(turn_id.is_none());
                assert!(tool_name.is_none());
            }
            other => panic!("expected ToolCallEnd, got {other:?}"),
        }
    }
}
