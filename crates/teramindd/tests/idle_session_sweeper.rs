//! Integration tests for the idle session sweeper.

mod common;

use std::sync::Arc;
use std::time::Duration;
use teramind_db::repos::session::NewSession;
use teramind_db::repos::{AgentRepo, SessionRepo};
use teramindd::services::fs_watcher::WatchRegistry;
use teramindd::services::idle_session_sweeper::sweep_once;
use teramindd::services::session_manager::{ActiveSession, SessionManager};
use time::OffsetDateTime;

async fn make_registry() -> Arc<WatchRegistry> {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let gaps = Arc::new(std::sync::atomic::AtomicU64::new(0));
    Arc::new(WatchRegistry::new(tx, gaps))
}

#[tokio::test]
async fn stale_session_is_closed() {
    let pool = teramind_db::testing::fresh_pool().await.unwrap();
    let agent_repo = AgentRepo::new(pool.clone());
    let session_repo = SessionRepo::new(pool.clone());
    let sessions = SessionManager::new();
    let registry = make_registry().await;

    let agent = agent_repo.upsert("claude_code", None).await.unwrap();
    let long_ago = OffsetDateTime::now_utc() - time::Duration::days(4);
    let sid = session_repo
        .insert(NewSession {
            agent_id: agent.id,
            agent_session_id: None,
            cwd: "/tmp/stale",
            project_id: None,
            parent_session_id: None,
            git_head: None,
            git_branch: None,
            os: "linux",
            hostname: "test",
            user_login: "tester",
            started_at: long_ago,
            user_id: None,
            device_id: None,
        })
        .await
        .unwrap();

    sessions
        .start(ActiveSession {
            session_id: sid,
            cwd: "/tmp/stale".into(),
            agent_kind: "claude_code".into(),
            started_at: long_ago,
            last_activity: long_ago,
            last_turn_id: None,
        })
        .await;

    sweep_once(
        &sessions,
        &session_repo,
        &registry,
        Duration::from_secs(3 * 24 * 3600),
    )
    .await;

    assert!(
        sessions.get(sid).await.is_none(),
        "stale session must be removed from manager"
    );
}

#[tokio::test]
async fn fresh_session_is_not_closed() {
    let pool = teramind_db::testing::fresh_pool().await.unwrap();
    let agent_repo = AgentRepo::new(pool.clone());
    let session_repo = SessionRepo::new(pool.clone());
    let sessions = SessionManager::new();
    let registry = make_registry().await;

    let agent = agent_repo.upsert("claude_code", None).await.unwrap();
    let now = OffsetDateTime::now_utc();
    let sid = session_repo
        .insert(NewSession {
            agent_id: agent.id,
            agent_session_id: None,
            cwd: "/tmp/fresh",
            project_id: None,
            parent_session_id: None,
            git_head: None,
            git_branch: None,
            os: "linux",
            hostname: "test",
            user_login: "tester",
            started_at: now,
            user_id: None,
            device_id: None,
        })
        .await
        .unwrap();

    sessions
        .start(ActiveSession {
            session_id: sid,
            cwd: "/tmp/fresh".into(),
            agent_kind: "claude_code".into(),
            started_at: now,
            last_activity: now,
            last_turn_id: None,
        })
        .await;

    sweep_once(
        &sessions,
        &session_repo,
        &registry,
        Duration::from_secs(3 * 24 * 3600),
    )
    .await;

    assert!(
        sessions.get(sid).await.is_some(),
        "fresh session must survive the sweep"
    );
}
