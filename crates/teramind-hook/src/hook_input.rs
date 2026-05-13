use serde::{Deserialize, Serialize};

/// Parsed Claude hook event JSON. One variant per hook event type that Teramind cares about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "hook_event_name")]
pub enum HookInput {
    // Variants added in Tasks 3-8.
}
