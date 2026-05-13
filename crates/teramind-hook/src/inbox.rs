use teramind_core::types::ingest_event::EventEnvelope;
use std::path::PathBuf;

pub fn write_envelope(env: &EventEnvelope) -> std::io::Result<PathBuf> {
    let dir = inbox_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", env.client_event_id.0));
    std::fs::write(&path, serde_json::to_vec(env)?)?;
    Ok(path)
}

fn inbox_dir() -> PathBuf {
    #[cfg(unix)] {
        let home = std::env::var_os("HOME").map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("teramind").join("inbox")
    }
    #[cfg(windows)] {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Temp"))
            .join("teramind").join("inbox")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use teramind_core::ids::{ClientEventId, SessionId};
    use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
    use time::OffsetDateTime;

    #[test]
    fn writes_envelope_to_inbox() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        std::env::set_var("XDG_DATA_HOME", tmp.path().join("xdg-data"));
        #[cfg(windows)] std::env::set_var("LOCALAPPDATA", tmp.path());
        let env = EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::now_utc(),
            event: IngestEvent::UserPrompt {
                session_id: SessionId::new(), turn_ordinal: 0, prompt: "x".into(), turn_id: None,
            },
        };
        let path = write_envelope(&env).unwrap();
        assert!(path.exists());
        let parsed: EventEnvelope = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(parsed.client_event_id, env.client_event_id);
    }
}
