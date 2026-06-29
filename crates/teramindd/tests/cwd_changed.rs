//! Integration tests for CwdChanged fs-watch logic.
//!
//! Verifies that going outside the session root registers a new watch,
//! returning inside unregisters it, and moving between external dirs
//! swaps the watch correctly.

mod common;

use teramind_core::ids::{ClientEventId, SessionId};
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use teramindd::services::fs_watcher::WatchRegistry;
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
            agent_version: None,
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

/// Poll until the registry reaches `expected` watch count (or 2 s timeout).
async fn wait_for(registry: &WatchRegistry, expected: usize) {
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(2);
    loop {
        if registry.watched_count().await == expected {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for watch count to reach {expected}"
        );
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    }
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
    wait_for(&h.registry, 1).await;

    h.ingest
        .try_enqueue(cwd_changed_envelope(sid, &root, &external))
        .unwrap();
    wait_for(&h.registry, 2).await;
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
    wait_for(&h.registry, 1).await;

    h.ingest
        .try_enqueue(cwd_changed_envelope(sid, &root, &external))
        .unwrap();
    wait_for(&h.registry, 2).await;

    h.ingest
        .try_enqueue(cwd_changed_envelope(sid, &external, &root))
        .unwrap();
    wait_for(&h.registry, 1).await;
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
    wait_for(&h.registry, 1).await;

    h.ingest
        .try_enqueue(cwd_changed_envelope(sid, &root, &external1))
        .unwrap();
    wait_for(&h.registry, 2).await;

    h.ingest
        .try_enqueue(cwd_changed_envelope(sid, &external1, &external2))
        .unwrap();
    // external1 dropped, external2 added — still 2 total (root + external2).
    wait_for(&h.registry, 2).await;
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
    wait_for(&h.registry, 1).await;

    h.ingest
        .try_enqueue(cwd_changed_envelope(sid, &root, &sub))
        .unwrap();

    // Give the ingest worker time to process — count must stay at 1.
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    assert_eq!(h.registry.watched_count().await, 1);
}
