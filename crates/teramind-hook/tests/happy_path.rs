#![cfg(unix)]
use std::process::{Command, Stdio};
use std::io::Write;
use teramind_db::{pg_supervisor::PgSupervisor, pool::DbPool, migrate};
use teramind_ipc::transport::listen;
use std::sync::Arc;
use teramindd::services::{
    ingest::{IngestDeps, IngestService, IngestStats},
    ipc_server::{run_accept_loop, DaemonIpcHandler},
    jsonl_writer::JsonlWriter,
    session_manager::SessionManager,
};
use teramind_db::repos::{AgentRepo, DiffRepo, SearchRepo, SessionRepo, TraceRepo};
use teramind_core::redact::Redactor;
use tempfile::tempdir;

fn cargo_bin(name: &str) -> std::path::PathBuf {
    std::env::var(format!("CARGO_BIN_EXE_{name}")).map(Into::into)
        .unwrap_or_else(|_| {
            let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
            let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
            std::path::PathBuf::from(target).join(profile).join(name)
        })
}

#[tokio::test]
async fn hook_session_start_persists_to_postgres() {
    let _ = Command::new("cargo").args(["build", "-p", "teramind-hook"]).status();

    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().join("pg"), "teramind_test").await.unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();

    let jsonl = Arc::new(JsonlWriter::open(tmp.path().join("raw")).await.unwrap());
    let stats = Arc::new(IngestStats::default());
    let (raw_tx, _) = tokio::sync::mpsc::unbounded_channel();
    let registry = std::sync::Arc::new(
        teramindd::services::fs_watcher::WatchRegistry::new(
            raw_tx,
            std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        ),
    );
    let ingest = Arc::new(IngestService::spawn(64, IngestDeps {
        redactor: Arc::new(Redactor::with_default_rules()),
        jsonl: jsonl.clone(),
        sessions: SessionManager::new(),
        agents: AgentRepo::new(pool.clone()),
        session_repo: SessionRepo::new(pool.clone()),
        trace: TraceRepo::new(pool.clone()),
        diffs: DiffRepo::new(pool.clone()),
        stats: stats.clone(),
        dead_letter_dir: tmp.path().join("dl"),
        write_tool_ring: teramindd::services::write_tool_ring::WriteToolRing::new(
            64,
            time::Duration::seconds(5),
        ),
        fs_registry: registry,
    }));
    let handler = Arc::new(DaemonIpcHandler {
        ingest: ingest.clone(), stats: stats.clone(),
        started: std::time::Instant::now(),
        last_pg_bytes: 0.into(), last_jsonl_bytes: 0.into(),
        search_repo: SearchRepo::new(pool.clone()),
        jsonl_dir: tmp.path().join("raw"),
    });
    let sock = tmp.path().join("t.sock");
    let listener = listen(&sock).unwrap();
    let h2 = handler.clone();
    tokio::spawn(async move { let _ = run_accept_loop(listener, h2).await; });

    let hook = cargo_bin("teramind-hook");
    let payload = r#"{"hook_event_name":"SessionStart","session_id":"e2e-test","cwd":"/work","source":"startup"}"#;
    let mut child = Command::new(&hook)
        .env("TERAMIND_SOCKET", sock.to_string_lossy().to_string())
        .env("TERAMIND_HOOK_NO_SPAWN", "1")
        .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().unwrap();
    child.stdin.as_mut().unwrap().write_all(payload.as_bytes()).unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "hook exited non-zero");

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let expected_id = teramind_hook::translate::claude_session_to_uuid("e2e-test").0;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM sessions WHERE id=$1")
        .bind(expected_id).fetch_one(pool.pg()).await.unwrap();
    assert_eq!(count, 1, "expected exactly one session row with id={expected_id}");

    sup.shutdown().await.unwrap();
}

#[tokio::test]
async fn hook_tool_call_lifecycle_persists() {
    let _ = Command::new("cargo").args(["build", "-p", "teramind-hook"]).status();
    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().join("pg"), "teramind_test").await.unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();

    let jsonl = Arc::new(JsonlWriter::open(tmp.path().join("raw")).await.unwrap());
    let stats = Arc::new(IngestStats::default());
    let (raw_tx, _) = tokio::sync::mpsc::unbounded_channel();
    let registry = std::sync::Arc::new(
        teramindd::services::fs_watcher::WatchRegistry::new(
            raw_tx,
            std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        ),
    );
    let ingest = Arc::new(IngestService::spawn(64, IngestDeps {
        redactor: Arc::new(Redactor::with_default_rules()),
        jsonl: jsonl.clone(),
        sessions: SessionManager::new(),
        agents: AgentRepo::new(pool.clone()),
        session_repo: SessionRepo::new(pool.clone()),
        trace: TraceRepo::new(pool.clone()),
        diffs: DiffRepo::new(pool.clone()),
        stats: stats.clone(),
        dead_letter_dir: tmp.path().join("dl"),
        write_tool_ring: teramindd::services::write_tool_ring::WriteToolRing::new(
            64,
            time::Duration::seconds(5),
        ),
        fs_registry: registry,
    }));
    let handler = Arc::new(DaemonIpcHandler {
        ingest: ingest.clone(), stats: stats.clone(),
        started: std::time::Instant::now(),
        last_pg_bytes: 0.into(), last_jsonl_bytes: 0.into(),
        search_repo: SearchRepo::new(pool.clone()),
        jsonl_dir: tmp.path().join("raw"),
    });
    let sock = tmp.path().join("t.sock");
    let listener = listen(&sock).unwrap();
    let h2 = handler.clone();
    tokio::spawn(async move { let _ = run_accept_loop(listener, h2).await; });

    let hook = cargo_bin("teramind-hook");
    let sid = format!("tc-{}", uuid::Uuid::new_v4());
    let tmp_state = tempdir().unwrap();
    let payloads = vec![
        format!(r#"{{"hook_event_name":"SessionStart","session_id":"{sid}","cwd":"/w","source":"startup"}}"#),
        format!(r#"{{"hook_event_name":"UserPromptSubmit","session_id":"{sid}","cwd":"/w","prompt":"do it"}}"#),
        format!(r#"{{"hook_event_name":"PreToolUse","session_id":"{sid}","cwd":"/w","tool_name":"Edit","tool_input":{{"file_path":"/w/x.rs"}}}}"#),
        format!(r#"{{"hook_event_name":"PostToolUse","session_id":"{sid}","cwd":"/w","tool_name":"Edit","tool_input":{{"file_path":"/w/x.rs"}},"tool_response":"ok"}}"#),
    ];

    for p in &payloads {
        let mut child = Command::new(&hook)
            .env("TERAMIND_SOCKET", sock.to_string_lossy().to_string())
            .env("TERAMIND_HOOK_NO_SPAWN", "1")
            .env("XDG_DATA_HOME", tmp_state.path())
            .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null())
            .spawn().unwrap();
        child.stdin.as_mut().unwrap().write_all(p.as_bytes()).unwrap();
        assert!(child.wait().unwrap().success());
    }

    tokio::time::sleep(std::time::Duration::from_millis(700)).await;

    let session_uuid = teramind_hook::translate::claude_session_to_uuid(&sid).0;

    let (s_count,): (i64,) = sqlx::query_as("SELECT count(*) FROM sessions WHERE id=$1")
        .bind(session_uuid).fetch_one(pool.pg()).await.unwrap();
    assert_eq!(s_count, 1);

    let (t_count,): (i64,) = sqlx::query_as("SELECT count(*) FROM turns WHERE session_id=$1")
        .bind(session_uuid).fetch_one(pool.pg()).await.unwrap();
    assert_eq!(t_count, 1);

    let (tc_count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM tool_calls tc JOIN turns t ON tc.turn_id=t.id WHERE t.session_id=$1 AND tc.name='Edit' AND tc.output='ok'")
        .bind(session_uuid).fetch_one(pool.pg()).await.unwrap();
    assert_eq!(tc_count, 1);

    sup.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_session_start_emits_auto_recall_digest() {
    use std::io::Write;
    let _ = Command::new("cargo").args(["build", "-p", "teramind-hook"]).status();
    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().join("pg"), "teramind_test").await.unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();

    // Seed prior context: a session in cwd "/work-cwd" with one finalized turn.
    let agents = AgentRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await.unwrap();
    let sessions = SessionRepo::new(pool.clone());
    let now = time::OffsetDateTime::now_utc() - time::Duration::hours(1);
    let prior_sid = sessions.insert(teramind_db::repos::session::NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/work-cwd", project_id: None,
        parent_session_id: None, git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u", started_at: now,
    }).await.unwrap();
    let trace = TraceRepo::new(pool.clone());
    let prior_turn = trace.upsert_turn(prior_sid, 0, now, Some("yesterday's bug fix")).await.unwrap();
    trace.finalize_turn(prior_turn, now, Some("fixed by adjusting timeout"), None, None, None, None).await.unwrap();
    sqlx::query("REFRESH MATERIALIZED VIEW traces_fts").execute(pool.pg()).await.unwrap();

    let jsonl = Arc::new(JsonlWriter::open(tmp.path().join("raw")).await.unwrap());
    let stats = Arc::new(IngestStats::default());
    let (raw_tx, _) = tokio::sync::mpsc::unbounded_channel();
    let registry = std::sync::Arc::new(
        teramindd::services::fs_watcher::WatchRegistry::new(
            raw_tx,
            std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        ),
    );
    let ingest = Arc::new(IngestService::spawn(64, IngestDeps {
        redactor: Arc::new(Redactor::with_default_rules()),
        jsonl: jsonl.clone(), sessions: SessionManager::new(),
        agents: AgentRepo::new(pool.clone()), session_repo: SessionRepo::new(pool.clone()),
        trace: TraceRepo::new(pool.clone()), diffs: DiffRepo::new(pool.clone()),
        stats: stats.clone(), dead_letter_dir: tmp.path().join("dl"),
        write_tool_ring: teramindd::services::write_tool_ring::WriteToolRing::new(
            64,
            time::Duration::seconds(5),
        ),
        fs_registry: registry,
    }));
    let handler = Arc::new(DaemonIpcHandler {
        ingest: ingest.clone(), stats: stats.clone(),
        started: std::time::Instant::now(),
        last_pg_bytes: 0.into(), last_jsonl_bytes: 0.into(),
        search_repo: teramind_db::repos::SearchRepo::new(pool.clone()),
        jsonl_dir: tmp.path().join("raw"),
    });
    let sock = tmp.path().join("t.sock");
    let listener = listen(&sock).unwrap();
    let h2 = handler.clone();
    tokio::spawn(async move { let _ = run_accept_loop(listener, h2).await; });

    // Fire a SessionStart for a NEW session in the same cwd.
    let hook = cargo_bin("teramind-hook");
    let payload = r#"{"hook_event_name":"SessionStart","session_id":"new-sess","cwd":"/work-cwd","source":"startup"}"#;
    let mut child = Command::new(&hook)
        .env("TERAMIND_SOCKET", sock.to_string_lossy().to_string())
        .env("TERAMIND_HOOK_NO_SPAWN", "1")
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().unwrap();
    child.stdin.as_mut().unwrap().write_all(payload.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    assert!(stdout.contains("Recent Teramind context") || stdout.contains("yesterday"),
            "expected auto-recall digest on stdout; got: {stdout}");

    sup.shutdown().await.unwrap();
}
