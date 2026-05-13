use std::sync::Arc;
use teramind_core::ids::{ClientEventId, SessionId};
use teramind_core::redact::Redactor;
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use teramind_db::{pg_supervisor::PgSupervisor, pool::DbPool, migrate};
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo};
use teramindd::services::ingest::{IngestDeps, IngestService, IngestStats};
use teramindd::services::jsonl_writer::JsonlWriter;
use teramindd::services::session_manager::SessionManager;
use tempfile::tempdir;
use time::OffsetDateTime;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn ingest_drops_when_queue_is_saturated() {
    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().join("pg"), "teramind_test").await.unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();

    let jsonl = Arc::new(JsonlWriter::open(tmp.path().join("raw")).await.unwrap());
    let stats = Arc::new(IngestStats::default());
    let deps = IngestDeps {
        redactor: Arc::new(Redactor::with_default_rules()),
        jsonl, sessions: SessionManager::new(),
        agents: AgentRepo::new(pool.clone()),
        session_repo: SessionRepo::new(pool.clone()),
        trace: TraceRepo::new(pool.clone()),
        diffs: DiffRepo::new(pool.clone()),
        stats: stats.clone(),
        dead_letter_dir: tmp.path().join("dl"),
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
            event: IngestEvent::UserPrompt { session_id: sid, turn_ordinal: i as i32, prompt: format!("p{i}") },
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
