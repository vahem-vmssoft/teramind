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
        let _guard = crate::TEST_ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        // Save and restore env vars to avoid racing with tests that read them.
        let saved_home = std::env::var_os("HOME");
        let saved_xdg  = std::env::var_os("XDG_DATA_HOME");
        #[cfg(windows)] let saved_la = std::env::var_os("LOCALAPPDATA");

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
        let exists = path.exists();
        let parsed: EventEnvelope = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let id_matches = parsed.client_event_id == env.client_event_id;

        // Restore env vars before releasing the lock.
        match saved_home {
            Some(v) => std::env::set_var("HOME", v),
            None    => { let _ = std::env::remove_var("HOME"); }
        }
        match saved_xdg {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None    => { let _ = std::env::remove_var("XDG_DATA_HOME"); }
        }
        #[cfg(windows)]
        match saved_la {
            Some(v) => std::env::set_var("LOCALAPPDATA", v),
            None    => { let _ = std::env::remove_var("LOCALAPPDATA"); }
        }

        assert!(exists);
        assert!(id_matches);
    }
}
