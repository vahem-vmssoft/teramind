use crate::ids::{FileDiffId, SessionId, TurnId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attribution {
    Agent,
    Human,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    pub id: FileDiffId,
    pub turn_id: Option<TurnId>,
    pub session_id: SessionId,
    pub file_path: String,
    pub rel_path: String,
    pub attribution: Attribution,
    pub language: Option<String>,
    pub pre_excerpt: String,
    pub post_excerpt: String,
    pub unified_diff: String,
    #[serde(with = "serde_bytes_hex")]
    pub pre_hash: [u8; 32],
    #[serde(with = "serde_bytes_hex")]
    pub post_hash: [u8; 32],
    pub byte_size: i32,
    #[serde(with = "time::serde::rfc3339")]
    pub captured_at: OffsetDateTime,
}

mod serde_bytes_hex {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(v))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s = String::deserialize(d)?;
        let v = hex::decode(&s).map_err(serde::de::Error::custom)?;
        v.try_into().map_err(|_| serde::de::Error::custom("expected 32 bytes"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_diff_roundtrips_through_json() {
        let fd = FileDiff {
            id: FileDiffId::new(),
            turn_id: None,
            session_id: SessionId::new(),
            file_path: "/x.rs".to_string(),
            rel_path: "x.rs".to_string(),
            attribution: Attribution::Agent,
            language: Some("rust".to_string()),
            pre_excerpt: "a".to_string(),
            post_excerpt: "b".to_string(),
            unified_diff: "--- a\n+++ b\n".to_string(),
            pre_hash: [1u8; 32],
            post_hash: [2u8; 32],
            byte_size: 10,
            captured_at: OffsetDateTime::from_unix_timestamp(1_700_000_005).unwrap(),
        };
        let j = serde_json::to_string(&fd).unwrap();
        let back: FileDiff = serde_json::from_str(&j).unwrap();
        assert_eq!(fd, back);
    }
}
