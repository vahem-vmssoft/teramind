//! Integration tests for CwdChanged fs-watch logic.
//!
//! Verifies that going outside the session root registers a new watch,
//! returning inside unregisters it, and moving between external dirs
//! swaps the watch correctly.

mod common;

use teramind_core::ids::{ClientEventId, SessionId};
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use time::OffsetDateTime;

fn cwd_changed_envelope(session_id: SessionId, previous_cwd: &str, new_cwd: &str) -> EventEnvelope {
    EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: OffsetDateTime::now_utc(),
        event: IngestEvent::CwdChanged {
            session_id,
            previous_cwd: previous_cwd.into(),
            new_cwd: new_cwd.into(),
        },
    }
}

fn session_start_envelope(session_id: SessionId, cwd: &str) -> EventEnvelope {
    EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: OffsetDateTime::now_utc(),
        event: IngestEvent::SessionStart {
            session_id,
            agent_session_id: None,
            agent_kind: "claude_code".into(),
            cwd: cwd.into(),
            os: "linux".into(),
            hostname: "test".into(),
            user_login: "tester".into(),
            git_head: None,
            git_branch: None,
        },
    }
}

/// Helper: a real tmp dir that exists on disk so notify can watch it.
fn tmp_dir() -> (tempfile::TempDir, String) {
    let d = tempfile::tempdir().unwrap();
    let path = d
        .path()
        .canonicalize()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    (d, path)
}

#[tokio::test]
async fn going_outside_root_registers_new_watch() {
    let h = common::Harness::start().await.unwrap();

    let (_root_dir, root) = tmp_dir();
    let (_ext_dir, external) = tmp_dir();

    let sid = SessionId::new();
    h.ingest
        .try_enqueue(session_start_envelope(sid, &root))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // One watch: the root.
    assert_eq!(h.registry.watched_count().await, 1);

    h.ingest
        .try_enqueue(cwd_changed_envelope(sid, &root, &external))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Now two watches: root (still registered at SessionStart) + external.
    assert_eq!(h.registry.watched_count().await, 2);
}

#[tokio::test]
async fn returning_to_root_unregisters_external_watch() {
    let h = common::Harness::start().await.unwrap();

    let (_root_dir, root) = tmp_dir();
    let (_ext_dir, external) = tmp_dir();

    let sid = SessionId::new();
    h.ingest
        .try_enqueue(session_start_envelope(sid, &root))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    h.ingest
        .try_enqueue(cwd_changed_envelope(sid, &root, &external))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(h.registry.watched_count().await, 2);

    h.ingest
        .try_enqueue(cwd_changed_envelope(sid, &external, &root))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Back to just root.
    assert_eq!(h.registry.watched_count().await, 1);
}

#[tokio::test]
async fn moving_between_external_dirs_swaps_watch() {
    let h = common::Harness::start().await.unwrap();

    let (_root_dir, root) = tmp_dir();
    let (_ext1_dir, external1) = tmp_dir();
    let (_ext2_dir, external2) = tmp_dir();

    let sid = SessionId::new();
    h.ingest
        .try_enqueue(session_start_envelope(sid, &root))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    h.ingest
        .try_enqueue(cwd_changed_envelope(sid, &root, &external1))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(h.registry.watched_count().await, 2);

    h.ingest
        .try_enqueue(cwd_changed_envelope(sid, &external1, &external2))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // external1 dropped, external2 added — still 2 total (root + external2).
    assert_eq!(h.registry.watched_count().await, 2);
}

#[tokio::test]
async fn subdir_of_root_does_not_add_extra_watch() {
    let h = common::Harness::start().await.unwrap();

    let (root_dir, root) = tmp_dir();
    let sub = root_dir.path().join("src");
    std::fs::create_dir_all(&sub).unwrap();
    let sub = sub.canonicalize().unwrap().to_str().unwrap().to_string();

    let sid = SessionId::new();
    h.ingest
        .try_enqueue(session_start_envelope(sid, &root))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(h.registry.watched_count().await, 1);

    h.ingest
        .try_enqueue(cwd_changed_envelope(sid, &root, &sub))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // No extra watch — subdir is already covered by the root watcher.
    assert_eq!(h.registry.watched_count().await, 1);
}
