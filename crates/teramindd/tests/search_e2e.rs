use tempfile::tempdir;
use teramind_core::types::SearchRequest;
use teramind_db::repos::{AgentRepo, SearchRepo, SessionRepo, TraceRepo};
use teramindd::services::search;

#[tokio::test]
async fn do_search_finds_seeded_turn_via_fts() {
    let pool = teramind_db::testing::fresh_pool().await.unwrap();

    let agents = AgentRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await.unwrap();
    let sessions = SessionRepo::new(pool.clone());
    let now = time::OffsetDateTime::now_utc();
    let sid = sessions
        .insert(teramind_db::repos::session::NewSession {
            agent_id: agent.id,
            agent_session_id: None,
            cwd: "/w",
            project_id: None,
            parent_session_id: None,
            git_head: None,
            git_branch: None,
            os: "linux",
            hostname: "h",
            user_login: "u",
            started_at: now,
            user_id: None,
            device_id: None,
        })
        .await
        .unwrap();
    let trace = TraceRepo::new(pool.clone());
    let turn = trace
        .upsert_turn(sid, 0, now, Some("how to debug postgres replication lag"))
        .await
        .unwrap();
    trace
        .finalize_turn(
            turn,
            now,
            Some("the replication lag means the standby is behind"),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    sqlx::query("REFRESH MATERIALIZED VIEW traces_fts")
        .execute(pool.pg())
        .await
        .unwrap();

    let repo = SearchRepo::new(pool.clone());
    let req = SearchRequest {
        query: "replication lag".into(),
        limit: 10,
    };
    let out = search::do_search(
        &repo,
        None,
        "null:null",
        search::BlendWeights::default(),
        &req,
    )
    .await
    .unwrap();
    assert!(!out.hits.is_empty());
    assert!(!out.degraded);
}

#[tokio::test]
async fn do_search_falls_back_to_grep_when_pg_dies() {
    use std::io::Write;
    use teramind_core::ids::{ClientEventId, SessionId};
    use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};

    let tmp = tempdir().unwrap();

    // Seed a JSONL file for the grep fallback.
    let jsonl_dir = tmp.path().join("raw");
    std::fs::create_dir_all(&jsonl_dir).unwrap();
    let path = jsonl_dir.join("2026-05-13.jsonl");
    let env = EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: time::OffsetDateTime::now_utc(),
        event: IngestEvent::UserPrompt {
            session_id: SessionId::new(),
            turn_ordinal: 0,
            prompt: "fallback works for grep".into(),
            turn_id: None,
        },
    };
    let body = serde_json::to_vec(&env).unwrap();
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(&body).unwrap();
    writeln!(f).unwrap();

    // Use a pool that points at a non-listening port to simulate DB unavailability.
    let dead_pool = teramind_db::pool::DbPool::connect(
        sqlx::postgres::PgConnectOptions::new()
            .host("127.0.0.1")
            .port(1) // nothing listening here
            .database("ghost"),
    )
    .await;
    // If the pool itself fails to connect, we still want to test the fallback path.
    // Wrap a valid pool and expect degraded results from JSONL grep.
    let pool = teramind_db::testing::fresh_pool().await.unwrap();

    // Exhaust all connections by issuing an unreachable query to a dead pool.
    // Instead, just pass the live pool but kill its underlying connections via
    // pg_terminate_backend so queries fail. That approach is invasive. Instead,
    // simulate the scenario by providing a repo whose underlying pool points to a
    // disconnected database via invalid credentials.
    let _ = dead_pool; // drop

    // Actually, we use the do_search_with_fallback which gracefully handles
    // SqlxErrors. We point it at a working pool but pass a jsonl_dir that has data.
    // The function first tries PG (which succeeds), so hits.degraded=false.
    // To actually test degradation here without killing the shared PG, we use a
    // pool that can't connect. Since pool::connect() may succeed (lazy), we accept
    // that this test verifies the fallback path via the dead pool.
    let broken_pool = teramind_db::pool::DbPool::connect(
        sqlx::postgres::PgConnectOptions::new()
            .host("127.0.0.1")
            .port(1)
            .database("ghost")
            .username("nobody"),
    )
    .await
    .unwrap_or_else(|_| pool.clone()); // if can't even build pool, reuse working one

    // Shut down the pool's underlying connections by replacing with a disconnected one.
    // Simplest: just use the broken pool directly in the repo.
    let repo = SearchRepo::new(broken_pool.clone());
    let out = teramindd::services::search::do_search_with_fallback(
        &repo,
        &jsonl_dir,
        None,
        "null:null",
        teramindd::services::search::BlendWeights::default(),
        &SearchRequest {
            query: "fallback".into(),
            limit: 10,
        },
    )
    .await;
    assert!(out.degraded, "expected degraded result");
    assert!(!out.hits.is_empty(), "expected grep hit to come through");
}

#[tokio::test]
async fn ipc_search_request_returns_search_results() {
    use std::sync::Arc;
    use teramind_core::redact::Redactor;
    use teramind_db::repos::{AgentRepo, DiffRepo, SearchRepo, SessionRepo, TraceRepo};
    use teramind_ipc::client::{IpcClient, StreamClient};
    use teramind_ipc::proto::{Request, Response};
    use teramind_ipc::transport::{connect, listen};
    use teramindd::services::ingest::{IngestDeps, IngestService, IngestStats};
    use teramindd::services::ipc_server::{run_accept_loop, DaemonIpcHandler};
    use teramindd::services::jsonl_writer::JsonlWriter;
    use teramindd::services::session_manager::SessionManager;

    let tmp = tempdir().unwrap();
    let pool = teramind_db::testing::fresh_pool().await.unwrap();

    let agents = AgentRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await.unwrap();
    let sessions = SessionRepo::new(pool.clone());
    let now = time::OffsetDateTime::now_utc();
    let sid = sessions
        .insert(teramind_db::repos::session::NewSession {
            agent_id: agent.id,
            agent_session_id: None,
            cwd: "/w",
            project_id: None,
            parent_session_id: None,
            git_head: None,
            git_branch: None,
            os: "linux",
            hostname: "h",
            user_login: "u",
            started_at: now,
            user_id: None,
            device_id: None,
        })
        .await
        .unwrap();
    let trace = TraceRepo::new(pool.clone());
    let turn = trace
        .upsert_turn(sid, 0, now, Some("kafka consumer lag"))
        .await
        .unwrap();
    trace
        .finalize_turn(
            turn,
            now,
            Some("the kafka consumer was behind"),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    sqlx::query("REFRESH MATERIALIZED VIEW traces_fts")
        .execute(pool.pg())
        .await
        .unwrap();

    let jsonl = Arc::new(JsonlWriter::open(tmp.path().join("raw")).await.unwrap());
    let stats = Arc::new(IngestStats::default());
    let (raw_tx, _) = tokio::sync::mpsc::unbounded_channel();
    let registry = std::sync::Arc::new(teramindd::services::fs_watcher::WatchRegistry::new(
        raw_tx,
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
    ));
    let svc = IngestService::spawn(
        64,
        IngestDeps {
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
        },
    );
    let handler = Arc::new(DaemonIpcHandler {
        ingest: Arc::new(svc),
        stats: stats.clone(),
        started: std::time::Instant::now(),
        last_pg_bytes: 0.into(),
        last_jsonl_bytes: 0.into(),
        search_repo: SearchRepo::new(pool.clone()),
        jsonl_dir: tmp.path().join("raw"),
        embed_provider: Arc::new(teramindd::services::embed::NullEmbeddingProvider),
        embed_model: "null:null".into(),
        search_weights: teramindd::services::search::BlendWeights::default(),
        embed_stats: std::sync::Arc::new(
            teramindd::services::embedding_worker::EmbeddingStats::default(),
        ),
        pool: pool.clone(),
        wiki_repo: teramind_db::repos::WikiRepo::new(pool.clone()),
        summary_provider: std::sync::Arc::new(
            teramindd::services::summarize::null::NullSummaryProvider,
        ),
        summary_model: "test:null".into(),
        summarizer_stats: std::sync::Arc::new(
            teramindd::services::summarizer_worker::SummarizerStats::default(),
        ),
        decision_cache: None,
        team_share_writer: None,
    });
    let sock = tmp.path().join("t.sock");
    let listener = listen(&sock).unwrap();
    let h2 = handler.clone();
    tokio::spawn(async move {
        let _ = run_accept_loop(listener, h2).await;
    });

    let stream = connect(&sock).await.unwrap();
    let mut client = StreamClient::new(stream);
    let r = client
        .request(Request::Search(teramind_core::types::SearchRequest {
            query: "kafka".into(),
            limit: 10,
        }))
        .await
        .unwrap();
    match r {
        Response::SearchResults(sr) => {
            assert!(!sr.hits.is_empty(), "expected at least one hit");
            assert!(!sr.degraded);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}
