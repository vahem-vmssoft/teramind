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
    CwdChanged {
        session_id: SessionId,
        previous_cwd: String,
        new_cwd: String,
    },
    FileDiff {
        session_id: SessionId,
        #[serde(default)]
        turn_id: Option<TurnId>,
        file_path: String,
        rel_path: String,
        attribution: crate::types::file_diff::Attribution,
        #[serde(default)]
        language: Option<String>,
        pre_excerpt: String,
        post_excerpt: String,
        unified_diff: String,
        #[serde(with = "hex_array_32")]
        pre_hash: [u8; 32],
        #[serde(with = "hex_array_32")]
        post_hash: [u8; 32],
        byte_size: i32,
    },
}

mod hex_array_32 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(v))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s = String::deserialize(d)?;
        let v = hex::decode(&s).map_err(serde::de::Error::custom)?;
        v.try_into()
            .map_err(|_| serde::de::Error::custom("expected 32 bytes"))
    }
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
            IngestEvent::ToolCallEnd {
                session_id,
                turn_id,
                tool_name,
                ..
            } => {
                assert!(session_id.is_none());
                assert!(turn_id.is_none());
                assert!(tool_name.is_none());
            }
            other => panic!("expected ToolCallEnd, got {other:?}"),
        }
    }

    #[test]
    fn file_diff_event_roundtrips() {
        let env = EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::from_unix_timestamp(1_700_000_030).unwrap(),
            event: IngestEvent::FileDiff {
                session_id: SessionId::new(),
                turn_id: Some(TurnId::new()),
                file_path: "/proj/src/foo.rs".into(),
                rel_path: "src/foo.rs".into(),
                attribution: crate::types::file_diff::Attribution::Agent,
                language: Some("rust".into()),
                pre_excerpt: "fn old() {}".into(),
                post_excerpt: "fn new() {}".into(),
                unified_diff: "@@ -1 +1 @@\n-fn old() {}\n+fn new() {}\n".into(),
                pre_hash: [0u8; 32],
                post_hash: [1u8; 32],
                byte_size: 12,
            },
        };
        let j = serde_json::to_string(&env).unwrap();
        let back: EventEnvelope = serde_json::from_str(&j).unwrap();
        assert_eq!(env, back);
    }
}
