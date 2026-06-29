mod common;

use common::Harness;
use std::os::unix::fs::PermissionsExt;
use teramind_core::ids::{ClientEventId, SessionId};
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use time::OffsetDateTime;

/// Regression test for the EACCES fallback in `watch_or_fallback`.
///
/// Before the fix, `notify::watch(&cwd, Recursive)` failing with EACCES (e.g.
/// because a root-owned subdirectory exists) caused the whole WatchRegistry
/// entry to be skipped — no file diffs captured at all.
///
/// After the fix, the watcher falls back to hybrid non-recursive/recursive
/// coverage: the inaccessible subtree is skipped, but accessible subtrees
/// still emit diffs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn watcher_survives_inaccessible_subdir_and_still_emits_diffs() -> anyhow::Result<()> {
    let h = Harness::start().await?;
    let proj = h._tmp.path().canonicalize()?.join("proj");

    // Accessible subtree where we'll verify the watcher still works.
    let src = proj.join("src");
    std::fs::create_dir_all(&src)?;
    std::fs::write(src.join("main.rs"), "fn old() {}\n")?;

    // Inaccessible subtree: triggers EACCES during the Recursive watch scan.
    let logs = proj.join(".logs");
    std::fs::create_dir_all(&logs)?;
    std::fs::set_permissions(&logs, std::fs::Permissions::from_mode(0o000))?;

    // If we can still read the directory, we're running as root — chmod 000
    // is ineffective and the test would give a false pass. Skip instead.
    if std::fs::read_dir(&logs).is_ok() {
        let _ = std::fs::set_permissions(&logs, std::fs::Permissions::from_mode(0o755));
        eprintln!("skipping fs_watcher_eacces: running as root, chmod 000 has no effect");
        return Ok(());
    }

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
        .map_err(|_| anyhow::anyhow!("enqueue SessionStart"))?;

    // Wait for the watcher to register.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // Modify a file in the accessible subtree.
    std::fs::write(src.join("main.rs"), "fn new() {}\n")?;

    // Poll for the diff row (budget: 2s).
    let mut found = false;
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM file_diffs")
            .fetch_one(h.pool.pg())
            .await?;
        if count > 0 {
            found = true;
            break;
        }
    }

    // Restore permissions so tempdir cleanup can remove the directory.
    let _ = std::fs::set_permissions(&logs, std::fs::Permissions::from_mode(0o755));

    assert!(
        found,
        "no file_diffs row after 2s — watcher likely aborted on EACCES instead of falling back"
    );
    Ok(())
}
