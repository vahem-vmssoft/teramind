use std::sync::atomic::Ordering;
use std::sync::Arc;
use tempfile::tempdir;
use teramind_core::ids::{ClientEventId, SessionId};
use teramind_core::redact::Redactor;
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo};
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramindd::services::ingest::{IngestDeps, IngestService, IngestStats};
use teramindd::services::jsonl_writer::JsonlWriter;
use teramindd::services::session_manager::SessionManager;
use time::OffsetDateTime;

#[tokio::test]
async fn ingest_drops_when_queue_is_saturated() {
    let tmp = tempdir().unwrap();
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
    let deps = IngestDeps {
        redactor: Arc::new(Redactor::with_default_rules()),
        jsonl,
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
    };
    let svc = IngestService::spawn(4, deps);

    let sid = SessionId::new();
    let now = OffsetDateTime::now_utc();
    let mut accepted = 0u32;
    let mut dropped = 0u32;
    for i in 0..100 {
        let env = EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: now + time::Duration::milliseconds(i),
            event: IngestEvent::UserPrompt {
                session_id: sid,
                turn_ordinal: i as i32,
                prompt: format!("p{i}"),
                turn_id: None,
            },
        };
        match svc.try_enqueue(env) {
            Ok(_) => accepted += 1,
            Err(_) => dropped += 1,
        }
    }
    assert!(dropped > 0, "expected at least some drops with capacity=4");
    assert_eq!(stats.drops.load(Ordering::Relaxed) as u32, dropped);
    assert!(accepted + dropped == 100);

    sup.shutdown().await.unwrap();
}
