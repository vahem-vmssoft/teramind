use crate::ids::{SessionId, SkillId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    Authored,
    Codified,
    Imported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    pub id: SkillId,
    pub name: String,
    pub description: String,
    pub body: String,
    pub source: SkillSource,
    pub source_session_ids: Vec<SessionId>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skill_roundtrips_through_json() {
        let s = Skill {
            id: SkillId::new(),
            name: "kebab-name".into(),
            description: "desc".into(),
            body: "body".into(),
            source: SkillSource::Authored,
            source_session_ids: vec![],
            created_at: OffsetDateTime::from_unix_timestamp(1_700_000_006).unwrap(),
            updated_at: OffsetDateTime::from_unix_timestamp(1_700_000_006).unwrap(),
        };
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(s, serde_json::from_str::<Skill>(&j).unwrap());
    }
}
