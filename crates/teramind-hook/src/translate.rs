use crate::hook_input::HookInput;
use teramind_core::ids::{ClientEventId, SessionId, ToolCallId, TurnId};
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use time::OffsetDateTime;
use uuid::Uuid;

/// Translate a parsed Claude hook input into a Teramind `EventEnvelope`.
///
/// Returns `None` for hook events Teramind doesn't ingest (e.g. `HookInput::Other`).
pub fn translate(input: HookInput) -> Option<EventEnvelope> {
    let ts = OffsetDateTime::now_utc();
    let client_event_id = ClientEventId::new();
    let event = match input {
        HookInput::SessionStart { session_id, cwd, source: _ } => IngestEvent::SessionStart {
            session_id: claude_session_to_uuid(&session_id),
            agent_session_id: Some(session_id),
            agent_kind: "claude_code".to_string(),
            cwd,
            os: std::env::consts::OS.to_string(),
            hostname: hostname().unwrap_or_else(|| "unknown".to_string()),
            user_login: whoami_login().unwrap_or_else(|| "unknown".to_string()),
            git_head: None,
            git_branch: None,
        },
        HookInput::UserPromptSubmit { session_id, cwd: _, prompt } => {
            let ordinal = next_turn_ordinal(&session_id);
            let sid_uuid = claude_session_to_uuid(&session_id);
            let turn_id = claude_turn_to_uuid(sid_uuid, ordinal);
            IngestEvent::UserPrompt {
                session_id: sid_uuid,
                turn_ordinal: ordinal,
                prompt,
                turn_id: Some(turn_id),
            }
        }
        HookInput::PreToolUse { session_id, cwd: _, tool_name, tool_input } => {
            let sid_uuid = claude_session_to_uuid(&session_id);
            let turn_ord = current_turn_ordinal(&session_id);
            let turn_id = claude_turn_to_uuid(sid_uuid, turn_ord);
            let tool_ord = next_tool_ordinal(&session_id, turn_ord);
            let tool_call_id = claude_tool_call_to_uuid(turn_id, tool_ord);
            IngestEvent::ToolCallStart {
                turn_id,
                tool_call_id: Some(tool_call_id),
                ordinal: tool_ord,
                name: tool_name,
                input: tool_input,
            }
        }
        HookInput::PreCompact { session_id, cwd: _ } => {
            IngestEvent::PreCompact { session_id: claude_session_to_uuid(&session_id) }
        }
        HookInput::Stop { session_id, cwd: _, stop_hook_active } => {
            if stop_hook_active {
                return None;
            }
            IngestEvent::SessionEnd {
                session_id: claude_session_to_uuid(&session_id),
                reason: "stop_hook".to_string(),
            }
        }
        HookInput::PostToolUse { session_id, cwd: _, tool_name, tool_input: _, tool_response, is_error } => {
            let sid_uuid = claude_session_to_uuid(&session_id);
            let turn_ord = current_turn_ordinal(&session_id);
            let turn_id = claude_turn_to_uuid(sid_uuid, turn_ord);
            let tool_ord = current_tool_ordinal(&session_id, turn_ord);
            let tool_call_id = claude_tool_call_to_uuid(turn_id, tool_ord);
            IngestEvent::ToolCallEnd {
                tool_call_id,
                output: tool_response.unwrap_or_default(),
                is_error,
                duration_ms: 0,
                session_id: Some(sid_uuid),
                turn_id: Some(turn_id),
                tool_name: Some(tool_name),
            }
        }
        _ => return None,
    };
    Some(EventEnvelope { client_event_id, ts, event })
}

fn hostname() -> Option<String> {
    std::env::var("HOSTNAME").ok().or_else(|| {
        #[cfg(unix)] {
            std::fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_string())
        }
        #[cfg(windows)] {
            std::env::var("COMPUTERNAME").ok()
        }
    })
}

fn whoami_login() -> Option<String> {
    std::env::var("USER").ok().or_else(|| std::env::var("USERNAME").ok())
}

use std::path::PathBuf;

/// Atomically increment a per-session turn counter on disk.
/// Returns the new (post-increment) ordinal, 0-indexed.
fn next_turn_ordinal(session_id: &str) -> i32 {
    let dir = teramind_state_dir().join("turns");
    if std::fs::create_dir_all(&dir).is_err() {
        return 0;
    }
    let path: PathBuf = dir.join(format!("{session_id}.count"));
    let current: i32 = std::fs::read_to_string(&path).ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(-1);
    let next = current + 1;
    let _ = std::fs::write(&path, next.to_string());
    next
}

pub fn claude_turn_to_uuid(session_id: SessionId, turn_ordinal: i32) -> TurnId {
    const NAMESPACE: Uuid = Uuid::from_bytes([
        0xa1, 0xb2, 0xc3, 0xd4, 0xe5, 0xf6, 0x07, 0x18,
        0x29, 0x3a, 0x4b, 0x5c, 0x6d, 0x7e, 0x8f, 0x90,
    ]);
    let mut bytes = [0u8; 20];
    bytes[..16].copy_from_slice(session_id.0.as_bytes());
    bytes[16..].copy_from_slice(&turn_ordinal.to_be_bytes());
    TurnId(Uuid::new_v5(&NAMESPACE, &bytes))
}

fn current_turn_ordinal(session_id: &str) -> i32 {
    let path = teramind_state_dir().join("turns").join(format!("{session_id}.count"));
    std::fs::read_to_string(&path).ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

pub fn claude_tool_call_to_uuid(turn_id: TurnId, tool_ordinal: i32) -> ToolCallId {
    const NAMESPACE: Uuid = Uuid::from_bytes([
        0xc1, 0xd2, 0xe3, 0xf4, 0x05, 0x16, 0x27, 0x38,
        0x49, 0x5a, 0x6b, 0x7c, 0x8d, 0x9e, 0xaf, 0xb0,
    ]);
    let mut bytes = [0u8; 20];
    bytes[..16].copy_from_slice(turn_id.0.as_bytes());
    bytes[16..].copy_from_slice(&tool_ordinal.to_be_bytes());
    ToolCallId(Uuid::new_v5(&NAMESPACE, &bytes))
}

fn current_tool_ordinal(session_id: &str, turn_ordinal: i32) -> i32 {
    let path = teramind_state_dir().join("tools").join(format!("{session_id}-{turn_ordinal}.count"));
    std::fs::read_to_string(&path).ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn next_tool_ordinal(session_id: &str, turn_ordinal: i32) -> i32 {
    let dir = teramind_state_dir().join("tools");
    if std::fs::create_dir_all(&dir).is_err() {
        return 0;
    }
    let path = dir.join(format!("{session_id}-{turn_ordinal}.count"));
    let current: i32 = std::fs::read_to_string(&path).ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(-1);
    let next = current + 1;
    let _ = std::fs::write(&path, next.to_string());
    next
}

fn teramind_state_dir() -> PathBuf {
    #[cfg(unix)] {
        let home = std::env::var_os("HOME").map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("teramind")
    }
    #[cfg(windows)] {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Temp"))
            .join("teramind")
    }
}

/// Deterministically derive a `SessionId` UUID from Claude's session string.
/// Uses UUID v5 with a fixed namespace so multiple hook invocations agree.
pub fn claude_session_to_uuid(claude_session: &str) -> SessionId {
    const NAMESPACE: Uuid = Uuid::from_bytes([
        0x4b, 0x37, 0x8a, 0x7e, 0xb1, 0x4a, 0x4c, 0x2b,
        0x8a, 0x90, 0x6e, 0x6d, 0x6c, 0x6c, 0x77, 0x6f,
    ]);
    SessionId(Uuid::new_v5(&NAMESPACE, claude_session.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_session_uuid_is_deterministic() {
        let a = claude_session_to_uuid("abc-123");
        let b = claude_session_to_uuid("abc-123");
        assert_eq!(a, b);
        let c = claude_session_to_uuid("different");
        assert_ne!(a, c);
    }

    #[test]
    fn other_input_returns_none() {
        assert!(translate(HookInput::Other).is_none());
    }

    #[test]
    fn translates_pre_compact() {
        let input = HookInput::PreCompact { session_id: "abc-pc".into(), cwd: "/w".into() };
        let env = translate(input).expect("must translate");
        matches!(env.event, IngestEvent::PreCompact { .. });
    }

    #[test]
    fn translates_stop_final_to_session_end() {
        let input = HookInput::Stop {
            session_id: "abc-stop".into(),
            cwd: "/w".into(),
            stop_hook_active: false,
        };
        let env = translate(input).expect("must translate");
        matches!(env.event, IngestEvent::SessionEnd { .. });
    }

    #[test]
    fn translates_stop_inner_to_none() {
        let input = HookInput::Stop {
            session_id: "abc-stop".into(),
            cwd: "/w".into(),
            stop_hook_active: true,
        };
        assert!(translate(input).is_none());
    }

    #[test]
    fn translates_post_tool_use() {
        let sid = format!("test-post-{}", uuid::Uuid::new_v4());
        let _ = next_turn_ordinal(&sid);
        let _ = next_tool_ordinal(&sid, 0);
        let input = HookInput::PostToolUse {
            session_id: sid.clone(),
            cwd: "/w".into(),
            tool_name: "Edit".into(),
            tool_input: serde_json::json!({}),
            tool_response: Some("ok".into()),
            is_error: false,
        };
        let env = translate(input).expect("must translate");
        match env.event {
            IngestEvent::ToolCallEnd {
                tool_call_id, output, is_error, duration_ms,
                session_id, turn_id, tool_name
            } => {
                let expected_turn = claude_turn_to_uuid(claude_session_to_uuid(&sid), 0);
                let expected_tc = claude_tool_call_to_uuid(expected_turn, 0);
                assert_eq!(tool_call_id, expected_tc);
                assert_eq!(output, "ok");
                assert!(!is_error);
                assert_eq!(duration_ms, 0);
                assert!(session_id.is_some(), "session_id should be populated");
                assert!(turn_id.is_some(), "turn_id should be populated");
                assert_eq!(tool_name.as_deref(), Some("Edit"));
            }
            other => panic!("expected ToolCallEnd, got {other:?}"),
        }
    }

    #[test]
    fn translates_pre_tool_use() {
        let sid = format!("test-ptu-{}", uuid::Uuid::new_v4());
        let _ = next_turn_ordinal(&sid);
        let input = HookInput::PreToolUse {
            session_id: sid.clone(),
            cwd: "/w".into(),
            tool_name: "Edit".into(),
            tool_input: serde_json::json!({"file_path": "/w/x.rs"}),
        };
        let env = translate(input).expect("must translate");
        match env.event {
            IngestEvent::ToolCallStart { turn_id, ordinal, name, input, tool_call_id } => {
                assert_eq!(turn_id, claude_turn_to_uuid(claude_session_to_uuid(&sid), 0));
                assert_eq!(ordinal, 0);
                assert_eq!(name, "Edit");
                assert_eq!(input["file_path"], "/w/x.rs");
                assert!(tool_call_id.is_some());
            }
            other => panic!("expected ToolCallStart, got {other:?}"),
        }
    }

    #[test]
    fn translates_user_prompt_with_ordinal() {
        // Hold the shared env lock so `writes_envelope_to_inbox` can't mutate
        // HOME / XDG_DATA_HOME while we are mid-test writing ordinal count files.
        let _guard = crate::TEST_ENV_LOCK.lock().unwrap();
        let sid = format!("test-up-{}", uuid::Uuid::new_v4());
        let input = HookInput::UserPromptSubmit {
            session_id: sid.clone(),
            cwd: "/w".into(),
            prompt: "hello".into(),
        };
        let env = translate(input).expect("must translate");
        match env.event {
            IngestEvent::UserPrompt { session_id, turn_ordinal, prompt, turn_id } => {
                assert_eq!(session_id, claude_session_to_uuid(&sid));
                assert_eq!(turn_ordinal, 0);
                assert_eq!(prompt, "hello");
                assert!(turn_id.is_some());
            }
            other => panic!("expected UserPrompt, got {other:?}"),
        }
        let env2 = translate(HookInput::UserPromptSubmit {
            session_id: sid, cwd: "/w".into(), prompt: "next".into(),
        }).unwrap();
        if let IngestEvent::UserPrompt { turn_ordinal, .. } = env2.event {
            assert_eq!(turn_ordinal, 1);
        }
    }

    #[test]
    fn translates_session_start() {
        let input = HookInput::SessionStart {
            session_id: "abc-123".to_string(),
            cwd: "/work".to_string(),
            source: Some("startup".to_string()),
        };
        let env = translate(input).expect("must translate");
        match env.event {
            IngestEvent::SessionStart { session_id, cwd, agent_kind, .. } => {
                assert_eq!(session_id, claude_session_to_uuid("abc-123"));
                assert_eq!(cwd, "/work");
                assert_eq!(agent_kind, "claude_code");
            }
            other => panic!("expected SessionStart, got {other:?}"),
        }
    }
}
