use crate::ids::{FileDiffId, SessionId, SkillId, ToolCallId, TurnId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Hit {
    Turn      { turn_id: TurnId, session_id: SessionId, ordinal: i32, snippet: String, score: f32, #[serde(with = "time::serde::rfc3339")] ts: OffsetDateTime },
    ToolCall  { tool_call_id: ToolCallId, turn_id: TurnId, name: String, input_snippet: String, output_snippet: String, score: f32, #[serde(with = "time::serde::rfc3339")] ts: OffsetDateTime },
    FileDiff  { diff_id: FileDiffId, rel_path: String, hunk_snippet: String, score: f32, #[serde(with = "time::serde::rfc3339")] ts: OffsetDateTime },
    Skill     { skill_id: SkillId, name: String, body_snippet: String, score: f32 },
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hit_skill_variant_roundtrips() {
        let h = Hit::Skill { skill_id: SkillId::new(), name: "n".into(), body_snippet: "b".into(), score: 0.9 };
        assert_eq!(format!("{:?}", h), format!("{:?}", serde_json::from_str::<Hit>(&serde_json::to_string(&h).unwrap()).unwrap()));
    }
}
