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
