mod common;

use common::Harness;
use teramind_core::ids::{ClientEventId, SessionId};
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use time::OffsetDateTime;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn aws_key_in_diff_is_redacted_before_persist() -> anyhow::Result<()> {
    let h = Harness::start().await?;
    let proj = h._tmp.path().canonicalize()?.join("proj");
    std::fs::create_dir_all(&proj)?;
    std::fs::write(proj.join("creds.rs"), "let k = \"\";\n")?;

    let sid = SessionId::new();
    h.ingest
        .try_enqueue(EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::now_utc(),
            event: IngestEvent::SessionStart {
                session_id: sid,
                agent_session_id: None,
                agent_kind: "claude_code".into(),
                cwd: proj.to_string_lossy().to_string(),
                os: "linux".into(),
                hostname: "h".into(),
                user_login: "u".into(),
                git_head: None,
                git_branch: None,
            },
        })
        .map_err(|_| anyhow::anyhow!("enqueue"))?;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    std::fs::write(proj.join("creds.rs"), "let k = \"AKIAIOSFODNN7EXAMPLE\";\n")?;

    let mut excerpt: Option<String> = None;
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if let Some((post,)) = sqlx::query_as::<_, (String,)>(
            "SELECT post_excerpt FROM file_diffs ORDER BY captured_at DESC LIMIT 1",
        )
        .fetch_optional(h.pool.pg())
        .await?
        {
            excerpt = Some(post);
            break;
        }
    }
    let post = excerpt.expect("no diff row");
    assert!(
        !post.contains("AKIAIOSFODNN7EXAMPLE"),
        "redaction failed; post_excerpt: {post}"
    );
    assert!(
        post.contains("«redacted"),
        "expected redaction marker, got: {post}"
    );
    Ok(())
}
