//! summarizer §7 — `teramind sessions show <id>` prints the wiki page Markdown.
//!
//! Seeds: one finalized session + a wiki_pages row (matching the daemon's
//! summary_model db key "ollama:qwen3.6:latest", the default).

#![cfg(unix)]
use teramind_core::ids::SessionId;
use teramind_db::repos::{AgentRepo, SessionRepo, WikiRepo};

mod common;
use common::{boot_daemon, connect_daemon_db, stop_daemon};

const SUMMARY_MODEL: &str = "ollama:qwen3.6:latest";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sessions_show_by_id_prints_wiki_markdown() {
    if std::env::var("TERAMIND_TEST_PG_URL").is_err() {
        eprintln!("skipping: TERAMIND_TEST_PG_URL unset");
        return;
    }
    let h = boot_daemon();
    let pool = connect_daemon_db(&h).await.expect("connect to daemon DB");

    let session_id = SessionId::new();
    let body = format!("# Wiki body marker {}", uuid::Uuid::new_v4());

    // Seed: agent + finalized session + wiki page.
    let agents = AgentRepo::new(pool.clone());
    let agent = agents
        .upsert("claude-code", Some("1.0.0"))
        .await
        .expect("upsert agent");
    let sessions = SessionRepo::new(pool.clone());
    let now = time::OffsetDateTime::now_utc();
    sessions
        .insert_with_id(
            session_id,
            teramind_db::repos::session::NewSession {
                agent_id: agent.id,
                agent_session_id: None,
                cwd: "/tmp/show-by-id",
                project_id: None,
                parent_session_id: None,
                git_head: None,
                git_branch: None,
                os: "test",
                hostname: "test",
                user_login: "test",
                started_at: now,
                user_id: None,
                device_id: None,
            },
        )
        .await
        .expect("insert session");
    sessions.end(session_id, now, "test").await.expect("end");
    let wiki = WikiRepo::new(pool.clone());
    wiki.upsert(session_id, SUMMARY_MODEL, &body, 10, 20)
        .await
        .expect("upsert wiki");

    let out = h
        .cmd()
        .args(["sessions", "show", &session_id.0.to_string()])
        .output()
        .expect("exec teramind sessions show");
    stop_daemon(&h);

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "exit non-zero: stderr={}\nstdout={stdout}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains(&body),
        "stdout should contain wiki body; got:\n{stdout}"
    );
}
