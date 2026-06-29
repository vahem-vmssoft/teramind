mod common;

use common::Harness;
use teramind_core::ids::{ClientEventId, SessionId, ToolCallId, TurnId};
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use time::OffsetDateTime;

async fn count_diffs(pool: &teramind_db::pool::DbPool) -> i64 {
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM file_diffs")
        .fetch_one(pool.pg())
        .await
        .unwrap();
    n
}

async fn diff_row(
    pool: &teramind_db::pool::DbPool,
) -> Option<(String, String, Option<uuid::Uuid>)> {
    sqlx::query_as(
        "SELECT rel_path, attribution, turn_id FROM file_diffs ORDER BY captured_at DESC LIMIT 1",
    )
    .fetch_optional(pool.pg())
    .await
    .unwrap()
    .map(|(rel, attr, tid): (String, String, Option<uuid::Uuid>)| (rel, attr, tid))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agent_attribution_when_post_tool_use_precedes_write() -> anyhow::Result<()> {
    let h = Harness::start().await?;
    // Use canonicalized path to avoid macOS /var -> /private/var symlink issues.
    let proj = h._tmp.path().canonicalize()?.join("proj");
    std::fs::create_dir_all(&proj)?;
    std::fs::write(proj.join("a.rs"), "fn old() {}\n")?;

    let sid = SessionId::new();
    let tid = TurnId::new();

    // 1) SessionStart (registers the watcher)
    h.ingest
        .try_enqueue(EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::now_utc(),
            event: IngestEvent::SessionStart {
                session_id: sid,
                agent_session_id: None,
                agent_kind: "claude_code".into(),
                agent_version: None,
                cwd: proj.to_string_lossy().to_string(),
                os: "linux".into(),
                hostname: "h".into(),
                user_login: "u".into(),
                git_head: None,
                git_branch: None,
            },
        })
        .map_err(|_| anyhow::anyhow!("enqueue SessionStart"))?;

    // 2) UserPrompt creating the turn
    h.ingest
        .try_enqueue(EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::now_utc(),
            event: IngestEvent::UserPrompt {
                session_id: sid,
                turn_ordinal: 0,
                prompt: "edit a.rs".into(),
                turn_id: Some(tid),
            },
        })
        .map_err(|_| anyhow::anyhow!("enqueue UserPrompt"))?;

    // 3) ToolCallStart + ToolCallEnd for an Edit (records into the ring)
    let tool_id = ToolCallId::new();
    h.ingest
        .try_enqueue(EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::now_utc(),
            event: IngestEvent::ToolCallStart {
                turn_id: tid,
                tool_call_id: Some(tool_id),
                ordinal: 0,
                name: "Edit".into(),
                input: serde_json::json!({"path":"a.rs"}),
            },
        })
        .map_err(|_| anyhow::anyhow!("enqueue ToolStart"))?;
    h.ingest
        .try_enqueue(EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::now_utc(),
            event: IngestEvent::ToolCallEnd {
                tool_call_id: tool_id,
                output: "ok".into(),
                is_error: false,
                duration_ms: 5,
                session_id: Some(sid),
                turn_id: Some(tid),
                tool_name: Some("Edit".into()),
            },
        })
        .map_err(|_| anyhow::anyhow!("enqueue ToolEnd"))?;

    // Give ingest a moment to register the watcher + push to the ring.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // 4) Modify the file -> watcher should produce an Agent-attributed diff
    std::fs::write(proj.join("a.rs"), "fn new() {}\n")?;

    // Poll for the row to appear (budget: 1s per spec §9.7).
    let mut got = None;
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if count_diffs(&h.pool).await > 0 {
            got = diff_row(&h.pool).await;
            break;
        }
    }
    let (rel, attr, turn_id) = got.expect("file_diffs row not written within budget");
    assert_eq!(rel, "a.rs");
    assert_eq!(attr, "agent");
    assert_eq!(turn_id, Some(tid.0));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn human_attribution_when_no_recent_write_tool() -> anyhow::Result<()> {
    let h = Harness::start().await?;
    let proj = h._tmp.path().canonicalize()?.join("proj");
    std::fs::create_dir_all(&proj)?;
    std::fs::write(proj.join("a.rs"), "fn old() {}\n")?;

    let sid = SessionId::new();
    h.ingest
        .try_enqueue(EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::now_utc(),
            event: IngestEvent::SessionStart {
                session_id: sid,
                agent_session_id: None,
                agent_kind: "claude_code".into(),
                agent_version: None,
                cwd: proj.to_string_lossy().to_string(),
                os: "linux".into(),
                hostname: "h".into(),
                user_login: "u".into(),
                git_head: None,
                git_branch: None,
            },
        })
        .map_err(|_| anyhow::anyhow!("enqueue SessionStart"))?;

    // No tool events. Modify directly.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    std::fs::write(proj.join("a.rs"), "fn new() {}\n")?;

    let mut got = None;
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if count_diffs(&h.pool).await > 0 {
            got = diff_row(&h.pool).await;
            break;
        }
    }
    let (_, attr, turn_id) = got.expect("file_diffs row not written");
    assert_eq!(attr, "human");
    assert!(
        turn_id.is_none(),
        "human-attributed diff must not carry a turn_id"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ignored_paths_produce_no_file_diff_row() -> anyhow::Result<()> {
    let h = Harness::start().await?;
    let proj = h._tmp.path().canonicalize()?.join("proj");
    std::fs::create_dir_all(proj.join(".git"))?;
    std::fs::create_dir_all(proj.join("target"))?;
    std::fs::write(proj.join(".git/HEAD"), "ref: refs/heads/main\n")?;
    std::fs::write(proj.join("target/x"), "x")?;
    std::fs::write(proj.join("a.rs"), "fn old(){}\n")?;

    let sid = SessionId::new();
    h.ingest
        .try_enqueue(EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::now_utc(),
            event: IngestEvent::SessionStart {
                session_id: sid,
                agent_session_id: None,
                agent_kind: "claude_code".into(),
                agent_version: None,
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

    // Modify both an ignored and a tracked path.
    std::fs::write(proj.join(".git/HEAD"), "ref: refs/heads/feat\n")?;
    std::fs::write(proj.join("target/x"), "y")?;
    std::fs::write(proj.join("a.rs"), "fn new(){}\n")?;

    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    let rows: Vec<(String,)> = sqlx::query_as("SELECT rel_path FROM file_diffs")
        .fetch_all(h.pool.pg())
        .await?;
    let paths: Vec<String> = rows.into_iter().map(|(s,)| s).collect();
    assert!(
        paths.iter().any(|p| p == "a.rs"),
        "expected a.rs in {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.starts_with(".git/")),
        "got .git/ in {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.starts_with("target/")),
        "got target/ in {paths:?}"
    );
    Ok(())
}
