use std::sync::Arc;
use teramind_core::ids::{ClientEventId, SessionId};
use teramind_core::redact::Redactor;
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use teramind_db::{pg_supervisor::PgSupervisor, pool::DbPool, migrate};
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo};
use teramindd::services::ingest::{IngestService, IngestStats, IngestDeps, drain_inbox};
use teramindd::services::jsonl_writer::JsonlWriter;
use teramindd::services::session_manager::SessionManager;
use tempfile::tempdir;
use time::OffsetDateTime;

#[tokio::test]
async fn inbox_drainer_consumes_pending_files() {
    let tmp = tempdir().unwrap();
    let inbox = tmp.path().join("inbox"); std::fs::create_dir_all(&inbox).unwrap();

    let sid = SessionId::new();
    for i in 0..3 {
        let env = EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::now_utc(),
            event: IngestEvent::UserPrompt { session_id: sid, turn_ordinal: i, prompt: format!("p{i}") },
        };
        let path = inbox.join(format!("{}.json", env.client_event_id.0));
        std::fs::write(&path, serde_json::to_vec(&env).unwrap()).unwrap();
    }

    let sup = PgSupervisor::start(tmp.path().join("pg"), "teramind_test").await.unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();
    let jsonl = Arc::new(JsonlWriter::open(tmp.path().join("raw")).await.unwrap());
    let stats = Arc::new(IngestStats::default());
    let svc = IngestService::spawn(64, IngestDeps {
        redactor: Arc::new(Redactor::with_default_rules()),
        jsonl, sessions: SessionManager::new(),
        agents: AgentRepo::new(pool.clone()),
        session_repo: SessionRepo::new(pool.clone()),
        trace: TraceRepo::new(pool.clone()),
        diffs: DiffRepo::new(pool.clone()),
        stats: stats.clone(),
        dead_letter_dir: tmp.path().join("dl"),
    });

    let drained = drain_inbox(&inbox, &svc).await.unwrap();
    assert_eq!(drained, 3);
    let remaining = std::fs::read_dir(&inbox).unwrap().count();
    assert_eq!(remaining, 0);

    sup.shutdown().await.unwrap();
}
