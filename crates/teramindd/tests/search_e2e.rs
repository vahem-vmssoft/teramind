use teramind_core::types::SearchRequest;
use teramind_db::{pg_supervisor::PgSupervisor, pool::DbPool, migrate};
use teramind_db::repos::{AgentRepo, SessionRepo, TraceRepo, SearchRepo};
use teramindd::services::search;
use tempfile::tempdir;

#[tokio::test]
async fn do_search_finds_seeded_turn_via_fts() {
    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().join("pg"), "teramind_test").await.unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();

    let agents = AgentRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await.unwrap();
    let sessions = SessionRepo::new(pool.clone());
    let now = time::OffsetDateTime::now_utc();
    let sid = sessions.insert(teramind_db::repos::session::NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/w", project_id: None,
        parent_session_id: None, git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u", started_at: now,
    }).await.unwrap();
    let trace = TraceRepo::new(pool.clone());
    let turn = trace.upsert_turn(sid, 0, now, Some("how to debug postgres replication lag")).await.unwrap();
    trace.finalize_turn(turn, now, Some("the replication lag means the standby is behind"), None, None, None, None).await.unwrap();
    sqlx::query("REFRESH MATERIALIZED VIEW traces_fts").execute(pool.pg()).await.unwrap();

    let repo = SearchRepo::new(pool.clone());
    let req = SearchRequest { query: "replication lag".into(), limit: 10 };
    let out = search::do_search(&repo, &req).await.unwrap();
    assert!(!out.hits.is_empty());
    assert!(!out.degraded);

    sup.shutdown().await.unwrap();
}

#[tokio::test]
async fn do_search_falls_back_to_grep_when_pg_dies() {
    use std::io::Write;
    use teramind_core::ids::{ClientEventId, SessionId};
    use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().join("pg"), "teramind_test").await.unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();

    let jsonl_dir = tmp.path().join("raw"); std::fs::create_dir_all(&jsonl_dir).unwrap();
    let path = jsonl_dir.join("2026-05-13.jsonl");
    let env = EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: time::OffsetDateTime::now_utc(),
        event: IngestEvent::UserPrompt {
            session_id: SessionId::new(), turn_ordinal: 0, prompt: "fallback works for grep".into(), turn_id: None,
        },
    };
    let body = serde_json::to_vec(&env).unwrap();
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(&body).unwrap();
    writeln!(f).unwrap();

    sup.shutdown().await.unwrap();

    let repo = SearchRepo::new(pool.clone());
    let out = teramindd::services::search::do_search_with_fallback(
        &repo, &jsonl_dir, &SearchRequest { query: "fallback".into(), limit: 10 }
    ).await;
    assert!(out.degraded, "expected degraded result");
    assert!(!out.hits.is_empty(), "expected grep hit to come through");
}

#[tokio::test]
async fn ipc_search_request_returns_search_results() {
    use std::sync::Arc;
    use teramind_core::redact::Redactor;
    use teramind_ipc::client::{IpcClient, StreamClient};
    use teramind_ipc::proto::{Request, Response};
    use teramind_ipc::transport::{connect, listen};
    use teramindd::services::ingest::{IngestService, IngestStats, IngestDeps};
    use teramindd::services::jsonl_writer::JsonlWriter;
    use teramindd::services::session_manager::SessionManager;
    use teramindd::services::ipc_server::{DaemonIpcHandler, run_accept_loop};
    use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo, SearchRepo};

    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().join("pg"), "teramind_test").await.unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();

    let agents = AgentRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await.unwrap();
    let sessions = SessionRepo::new(pool.clone());
    let now = time::OffsetDateTime::now_utc();
    let sid = sessions.insert(teramind_db::repos::session::NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/w", project_id: None,
        parent_session_id: None, git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u", started_at: now,
    }).await.unwrap();
    let trace = TraceRepo::new(pool.clone());
    let turn = trace.upsert_turn(sid, 0, now, Some("kafka consumer lag")).await.unwrap();
    trace.finalize_turn(turn, now, Some("the kafka consumer was behind"), None, None, None, None).await.unwrap();
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
    let svc = IngestService::spawn(64, IngestDeps {
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
    });
    let handler = Arc::new(DaemonIpcHandler {
        ingest: Arc::new(svc), stats: stats.clone(),
        started: std::time::Instant::now(),
        last_pg_bytes: 0.into(), last_jsonl_bytes: 0.into(),
        search_repo: SearchRepo::new(pool.clone()),
        jsonl_dir: tmp.path().join("raw"),
    });
    let sock = tmp.path().join("t.sock");
    let listener = listen(&sock).unwrap();
    let h2 = handler.clone();
    tokio::spawn(async move { let _ = run_accept_loop(listener, h2).await; });

    let stream = connect(&sock).await.unwrap();
    let mut client = StreamClient::new(stream);
    let r = client.request(Request::Search(teramind_core::types::SearchRequest {
        query: "kafka".into(), limit: 10,
    })).await.unwrap();
    match r {
        Response::SearchResults(sr) => {
            assert!(!sr.hits.is_empty(), "expected at least one hit");
            assert!(!sr.degraded);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    sup.shutdown().await.unwrap();
}
