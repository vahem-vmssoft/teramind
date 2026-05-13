use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::io::AsyncWriteExt;
use teramind_core::types::ingest_event::EventEnvelope;
use time::macros::format_description;

pub struct JsonlWriter {
    base: PathBuf,
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    current_date: String,
    file: tokio::fs::File,
    path: PathBuf,
}

impl JsonlWriter {
    pub async fn open(base: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&base)?;
        let (date, path, file) = Self::open_today(&base).await?;
        Ok(Self {
            base,
            inner: Arc::new(Mutex::new(Inner { current_date: date, file, path })),
        })
    }

    async fn open_today(base: &PathBuf) -> std::io::Result<(String, PathBuf, tokio::fs::File)> {
        let now = time::OffsetDateTime::now_utc();
        let fmt = format_description!("[year]-[month]-[day]");
        let date = now.format(&fmt).expect("format date");
        let path = base.join(format!("{date}.jsonl"));
        let file = tokio::fs::OpenOptions::new().create(true).append(true).open(&path).await?;
        Ok((date, path, file))
    }

    pub async fn append(&self, env: &EventEnvelope) -> std::io::Result<()> {
        let mut g = self.inner.lock().await;
        let now = time::OffsetDateTime::now_utc();
        let fmt = format_description!("[year]-[month]-[day]");
        let today = now.format(&fmt).expect("format date");
        if today != g.current_date {
            let (d, p, f) = Self::open_today(&self.base).await?;
            g.current_date = d; g.file = f; g.path = p;
        }
        let mut bytes = serde_json::to_vec(env)?;
        bytes.push(b'\n');
        g.file.write_all(&bytes).await?;
        g.file.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use teramind_core::ids::{ClientEventId, SessionId};
    use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
    use tempfile::tempdir;
    use time::OffsetDateTime;

    #[tokio::test]
    async fn writer_appends_jsonl_to_daily_file() {
        let tmp = tempdir().unwrap();
        let w = JsonlWriter::open(tmp.path().to_path_buf()).await.unwrap();
        let env = EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::now_utc(),
            event: IngestEvent::UserPrompt {
                session_id: SessionId::new(), turn_ordinal: 0, prompt: "x".into(),
            },
        };
        w.append(&env).await.unwrap();
        w.append(&env).await.unwrap();
        let entries: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().collect();
        assert_eq!(entries.len(), 1);
        let p = entries[0].as_ref().unwrap().path();
        let body = std::fs::read_to_string(&p).unwrap();
        assert_eq!(body.lines().count(), 2);
    }
}
