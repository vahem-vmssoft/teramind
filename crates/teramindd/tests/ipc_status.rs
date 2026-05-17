#![cfg(unix)]
use std::sync::Arc;
use teramind_core::redact::Redactor;
use teramind_db::repos::{AgentRepo, DiffRepo, SearchRepo, SessionRepo, TraceRepo};
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramind_ipc::client::{IpcClient, StreamClient};
use teramind_ipc::proto::{Request, Response};
use teramind_ipc::transport::{connect, listen};
use teramindd::services::ingest::{IngestDeps, IngestService, IngestStats};
use teramindd::services::ipc_server::{run_accept_loop, DaemonIpcHandler};
use teramindd::services::jsonl_writer::JsonlWriter;
use teramindd::services::session_manager::SessionManager;

#[tokio::test]
async fn status_request_returns_status_report() {
    let tmp = tempfile::tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().join("pg"), "teramind_test")
        .await
        .unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();

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
    let r = client.request(Request::Status).await.unwrap();
    match r {
        Response::Status(s) => assert_eq!(s.ingest_drops_total, 0),
        other => panic!("unexpected: {:?}", other),
    }

    sup.shutdown().await.unwrap();
}
