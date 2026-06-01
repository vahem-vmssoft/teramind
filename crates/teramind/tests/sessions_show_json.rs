//! summarizer §7 — `teramind sessions show --json` emits a structured JSON
//! object {session_id, content, model, generated_at}.

#![cfg(unix)]
use teramind_core::ids::SessionId;
use teramind_db::repos::{AgentRepo, SessionRepo, WikiRepo};

mod common;
use common::{boot_daemon, connect_daemon_db, stop_daemon};

const SUMMARY_MODEL: &str = "ollama:qwen3.6:latest";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sessions_show_json_emits_expected_fields() {
    if std::env::var("TERAMIND_TEST_PG_URL").is_err() {
        eprintln!("skipping: TERAMIND_TEST_PG_URL unset");
        return;
    }
    let h = boot_daemon();
    let pool = connect_daemon_db(&h).await.expect("connect to daemon DB");

    let session_id = SessionId::new();
    let body = "# JSON body";

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
                cwd: "/tmp/show-json",
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
    WikiRepo::new(pool.clone())
        .upsert(session_id, SUMMARY_MODEL, body, 1, 2)
        .await
        .unwrap();

    let out = h
        .cmd()
        .args(["sessions", "show", &session_id.0.to_string(), "--json"])
        .output()
        .expect("exec teramind sessions show --json");
    stop_daemon(&h);

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "exit non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout not valid JSON ({e}):\n{stdout}"));
    assert!(v.get("session_id").and_then(|x| x.as_str()).is_some(), "session_id missing/non-string in {v}");
    assert!(v.get("content").and_then(|x| x.as_str()).is_some(), "content missing/non-string in {v}");
    assert!(v.get("model").and_then(|x| x.as_str()).is_some(), "model missing/non-string in {v}");
    assert!(v.get("generated_at").is_some(), "generated_at missing in {v}");
}
