use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "hook_event_name")]
pub enum HookInput {
    SessionStart {
        session_id: String,
        cwd: String,
        #[serde(default)]
        source: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_start_parses_from_real_payload() {
        let raw = r#"{
            "hook_event_name": "SessionStart",
            "session_id": "abc-123",
            "cwd": "/Users/me/project",
            "source": "startup"
        }"#;
        let parsed: HookInput = serde_json::from_str(raw).unwrap();
        match parsed {
            HookInput::SessionStart { session_id, cwd, source } => {
                assert_eq!(session_id, "abc-123");
                assert_eq!(cwd, "/Users/me/project");
                assert_eq!(source.as_deref(), Some("startup"));
            }
            other => panic!("expected SessionStart, got {other:?}"),
        }
    }
}
