use std::path::Path;
use teramind_core::ids::{ClientEventId, SessionId, TurnId};
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use teramind_core::types::Hit;
use tokio::process::Command;
use uuid::Uuid;

pub async fn run(jsonl_dir: &Path, query: &str, limit: u32) -> std::io::Result<Vec<Hit>> {
    if !jsonl_dir.exists() {
        return Ok(vec![]);
    }
    let output = Command::new("grep")
        .arg("-rIEn")
        .arg("--include=*.jsonl")
        .arg(query)
        .arg(jsonl_dir)
        .output()
        .await?;

    if !output.status.success() && output.status.code() != Some(1) {
        return Err(std::io::Error::other(format!(
            "grep failed: status={:?}",
            output.status
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut hits: Vec<Hit> = Vec::new();
    for line in stdout.lines().take(limit as usize * 4) {
        let (_path, rest) = match line.split_once(':') {
            Some(p) => p,
            None => continue,
        };
        let (_lineno, body) = match rest.split_once(':') {
            Some(p) => p,
            None => continue,
        };
        let env: EventEnvelope = match serde_json::from_str(body) {
            Ok(e) => e,
            Err(_) => continue,
        };
        match env.event {
            IngestEvent::UserPrompt {
                session_id,
                turn_ordinal,
                prompt,
                ..
            } => {
                hits.push(Hit::Turn {
                    turn_id: TurnId(Uuid::nil()),
                    session_id,
                    ordinal: turn_ordinal,
                    snippet: truncate_grep(&prompt, 200),
                    score: 0.5,
                    ts: env.ts,
                });
            }
            IngestEvent::AssistantTurn {
                turn_id,
                assistant_text,
                ..
            } => {
                hits.push(Hit::Turn {
                    turn_id,
                    session_id: SessionId(Uuid::nil()),
                    ordinal: -1,
                    snippet: truncate_grep(&assistant_text, 200),
                    score: 0.5,
                    ts: env.ts,
                });
            }
            _ => {}
        }
        if hits.len() >= limit as usize {
            break;
        }
    }
    let _ = ClientEventId::new();
    Ok(hits)
}

fn truncate_grep(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;
    use time::OffsetDateTime;

    #[tokio::test]
    async fn grep_finds_matching_user_prompt_line() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("2026-05-13.jsonl");
        let env = EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::now_utc(),
            event: IngestEvent::UserPrompt {
                session_id: SessionId::new(),
                turn_ordinal: 0,
                prompt: "stack overflow in serializer.rs:142".into(),
                turn_id: None,
            },
        };
        let line = serde_json::to_vec(&env).unwrap();
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&line).unwrap();
        writeln!(f).unwrap();

        let hits = run(tmp.path(), "serializer", 10).await.unwrap();
        assert!(!hits.is_empty(), "expected at least one grep hit");
        match &hits[0] {
            Hit::Turn { snippet, .. } => assert!(snippet.contains("serializer")),
            other => panic!("expected Turn, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn grep_returns_empty_for_missing_dir() {
        let hits = run(
            std::path::Path::new("/nonexistent/teramind/raw"),
            "anything",
            10,
        )
        .await
        .unwrap();
        assert!(hits.is_empty());
    }
}
