//! summarizer §7 — when a session exists but has no wiki_pages row, the CLI
//! prints an actionable pending/backlog message pointing at `teramind doctor`.

#![cfg(unix)]
use teramind_core::ids::SessionId;
use teramind_db::repos::{AgentRepo, SessionRepo};

mod common;
use common::{boot_daemon, connect_daemon_db, stop_daemon};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sessions_show_without_wiki_prints_pending_message() {
    if std::env::var("TERAMIND_TEST_PG_URL").is_err() {
        eprintln!("skipping: TERAMIND_TEST_PG_URL unset");
        return;
    }
    let h = boot_daemon();
    let pool = connect_daemon_db(&h).await.expect("connect to daemon DB");

    let session_id = SessionId::new();
    let agents = AgentRepo::new(pool.clone());
    let agent = agents.upsert("claude-code", Some("1.0.0")).await.unwrap();
    let sessions = SessionRepo::new(pool.clone());
    let now = time::OffsetDateTime::now_utc();
    sessions
        .insert_with_id(
            session_id,
            teramind_db::repos::session::NewSession {
                agent_id: agent.id,
                agent_session_id: None,
                cwd: "/tmp/no-wiki",
                project_id: None,
                parent_session_id: None,
                git_head: None,
                git_branch: None,
                os: "t",
                hostname: "t",
                user_login: "t",
                started_at: now,
                user_id: None,
                device_id: None,
            },
        )
        .await
        .unwrap();
    sessions.end(session_id, now, "test").await.unwrap();
    // Intentionally no wiki_pages row.

    let out = h
        .cmd()
        .args(["sessions", "show", &session_id.0.to_string()])
        .output()
        .expect("exec teramind sessions show");
    stop_daemon(&h);

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
    .to_lowercase();
    let actionable =
        combined.contains("pending") || combined.contains("backlog") || combined.contains("doctor");
    assert!(
        actionable,
        "stdout/stderr should mention pending/backlog/doctor; got:\n{combined}"
    );
}
