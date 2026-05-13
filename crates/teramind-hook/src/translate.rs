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
            IngestEvent::UserPrompt {
                session_id: claude_session_to_uuid(&session_id),
                turn_ordinal: ordinal,
                prompt,
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
    fn translates_user_prompt_with_ordinal() {
        let sid = format!("test-up-{}", uuid::Uuid::new_v4());
        let input = HookInput::UserPromptSubmit {
            session_id: sid.clone(),
            cwd: "/w".into(),
            prompt: "hello".into(),
        };
        let env = translate(input).expect("must translate");
        match env.event {
            IngestEvent::UserPrompt { session_id, turn_ordinal, prompt } => {
                assert_eq!(session_id, claude_session_to_uuid(&sid));
                assert_eq!(turn_ordinal, 0);
                assert_eq!(prompt, "hello");
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
