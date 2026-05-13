use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageStats {
    pub id: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub sampled_at: OffsetDateTime,
    pub pg_bytes: i64,
    pub jsonl_bytes: i64,
    pub session_count: i64,
    pub turn_count: i64,
    pub diff_count: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn storage_stats_roundtrips_through_json() {
        let s = StorageStats {
            id: 1,
            sampled_at: OffsetDateTime::from_unix_timestamp(1_700_000_007).unwrap(),
            pg_bytes: 100, jsonl_bytes: 200, session_count: 3, turn_count: 30, diff_count: 5,
        };
        assert_eq!(s, serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap());
    }
}
