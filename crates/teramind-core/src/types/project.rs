use crate::ids::ProjectId;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub root_path: String,
    pub git_remote: Option<String>,
    pub display_name: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub first_seen: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_roundtrips_through_json() {
        let p = Project {
            id: ProjectId::new(),
            root_path: "/home/dev/repo".to_string(),
            git_remote: Some("git@github.com:org/repo.git".to_string()),
            display_name: None,
            first_seen: OffsetDateTime::from_unix_timestamp(1_700_000_001).unwrap(),
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }
}
