use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

macro_rules! id_newtype {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
            pub fn nil() -> Self {
                Self(Uuid::nil())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id_newtype!(AgentId);
id_newtype!(ClientEventId);
id_newtype!(FileDiffId);
id_newtype!(ProjectId);
id_newtype!(SessionId);
id_newtype!(SkillId);
id_newtype!(ToolCallId);
id_newtype!(TurnId);
id_newtype!(WikiPageId);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_id_roundtrips_as_uuid_string() {
        let id = SessionId::new();
        let json = serde_json::to_string(&id).unwrap();
        let back: SessionId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
        assert_eq!(json, format!("\"{}\"", id.0));
    }
}
