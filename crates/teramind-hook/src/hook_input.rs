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
    UserPromptSubmit {
        session_id: String,
        cwd: String,
        prompt: String,
    },
    PreToolUse {
        session_id: String,
        cwd: String,
        tool_name: String,
        tool_input: serde_json::Value,
    },
    PostToolUse {
        session_id: String,
        cwd: String,
        tool_name: String,
        tool_input: serde_json::Value,
        #[serde(default, alias = "tool_output")]
        tool_response: Option<String>,
        #[serde(default)]
        is_error: bool,
    },
    Stop {
        session_id: String,
        cwd: String,
        #[serde(default)]
        stop_hook_active: bool,
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

    #[test]
    fn user_prompt_submit_parses() {
        let raw = r#"{
            "hook_event_name": "UserPromptSubmit",
            "session_id": "abc-123",
            "cwd": "/Users/me/project",
            "prompt": "Fix the failing test"
        }"#;
        let parsed: HookInput = serde_json::from_str(raw).unwrap();
        match parsed {
            HookInput::UserPromptSubmit { session_id, cwd, prompt } => {
                assert_eq!(session_id, "abc-123");
                assert_eq!(cwd, "/Users/me/project");
                assert_eq!(prompt, "Fix the failing test");
            }
            other => panic!("expected UserPromptSubmit, got {other:?}"),
        }
    }

    #[test]
    fn pre_tool_use_parses() {
        let raw = r#"{
            "hook_event_name": "PreToolUse",
            "session_id": "abc-123",
            "cwd": "/w",
            "tool_name": "Edit",
            "tool_input": { "file_path": "/w/x.rs", "old_string": "a", "new_string": "b" }
        }"#;
        let parsed: HookInput = serde_json::from_str(raw).unwrap();
        match parsed {
            HookInput::PreToolUse { session_id, tool_name, tool_input, .. } => {
                assert_eq!(session_id, "abc-123");
                assert_eq!(tool_name, "Edit");
                assert_eq!(tool_input["file_path"], "/w/x.rs");
            }
            other => panic!("expected PreToolUse, got {other:?}"),
        }
    }

    #[test]
    fn post_tool_use_parses_with_tool_response() {
        let raw = r#"{
            "hook_event_name": "PostToolUse",
            "session_id": "abc-123",
            "cwd": "/w",
            "tool_name": "Edit",
            "tool_input": { "file_path": "/w/x.rs" },
            "tool_response": "edited successfully"
        }"#;
        let parsed: HookInput = serde_json::from_str(raw).unwrap();
        match parsed {
            HookInput::PostToolUse { session_id, tool_name, tool_response, is_error, .. } => {
                assert_eq!(session_id, "abc-123");
                assert_eq!(tool_name, "Edit");
                assert_eq!(tool_response, Some("edited successfully".to_string()));
                assert_eq!(is_error, false);
            }
            other => panic!("expected PostToolUse, got {other:?}"),
        }
    }

    #[test]
    fn post_tool_use_parses_with_is_error() {
        let raw = r#"{
            "hook_event_name": "PostToolUse",
            "session_id": "abc-123",
            "cwd": "/w",
            "tool_name": "Bash",
            "tool_input": { "command": "false" },
            "tool_response": "exit 1",
            "is_error": true
        }"#;
        let parsed: HookInput = serde_json::from_str(raw).unwrap();
        match parsed {
            HookInput::PostToolUse { is_error, .. } => assert!(is_error),
            other => panic!("expected PostToolUse, got {other:?}"),
        }
    }

    #[test]
    fn stop_final_parses() {
        let raw = r#"{
            "hook_event_name": "Stop",
            "session_id": "abc-123",
            "cwd": "/w",
            "stop_hook_active": false
        }"#;
        let parsed: HookInput = serde_json::from_str(raw).unwrap();
        match parsed {
            HookInput::Stop { session_id, stop_hook_active, .. } => {
                assert_eq!(session_id, "abc-123");
                assert!(!stop_hook_active);
            }
            other => panic!("expected Stop, got {other:?}"),
        }
    }
}
