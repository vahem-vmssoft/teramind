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
async fn ingest_session_start_then_user_prompt_writes_rows() {
    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().join("pg"), "teramind_test")
        .await
        .unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();

    let jsonl = Arc::new(JsonlWriter::open(tmp.path().join("raw")).await.unwrap());
    let stats = Arc::new(IngestStats::default());
    let deps = IngestDeps {
        redactor: Arc::new(Redactor::with_default_rules()),
        jsonl: jsonl.clone(),
        sessions: SessionManager::new(),
        agents: AgentRepo::new(pool.clone()),
        session_repo: SessionRepo::new(pool.clone()),
        trace: TraceRepo::new(pool.clone()),
        diffs: DiffRepo::new(pool.clone()),
        stats: stats.clone(),
        dead_letter_dir: tmp.path().join("dl"),
    };
    let svc = IngestService::spawn(64, deps);

    let session_id = SessionId::new();
    let now = OffsetDateTime::now_utc();
    svc.try_enqueue(EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: now,
        event: IngestEvent::SessionStart {
            session_id,
            agent_session_id: Some("abc".into()),
            agent_kind: "claude_code".into(),
            cwd: "/w".into(),
            os: "linux".into(),
            hostname: "h".into(),
            user_login: "u".into(),
            git_head: None,
            git_branch: None,
        },
    })
    .unwrap();
    svc.try_enqueue(EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: now + time::Duration::seconds(1),
        event: IngestEvent::UserPrompt {
            session_id,
            turn_ordinal: 0,
            prompt: "hi key=AKIAIOSFODNN7EXAMPLE end".into(),
            turn_id: None,
        },
    })
    .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let (turn_count,): (i64,) = sqlx::query_as("SELECT count(*) FROM turns")
        .fetch_one(pool.pg())
        .await
        .unwrap();
    assert_eq!(turn_count, 1);
    let (prompt,): (Option<String>,) = sqlx::query_as("SELECT user_prompt FROM turns LIMIT 1")
        .fetch_one(pool.pg())
        .await
        .unwrap();
    let prompt = prompt.unwrap();
    assert!(
        !prompt.contains("AKIAIOSFODNN7EXAMPLE"),
        "secret leaked: {prompt}"
    );

    sup.shutdown().await.unwrap();
}

#[tokio::test]
async fn storage_stats_sampler_records_a_row() {
    let tmp = tempfile::tempdir().unwrap();
    let sup =
        teramind_db::pg_supervisor::PgSupervisor::start(tmp.path().join("pg"), "teramind_test")
            .await
            .unwrap();
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options())
        .await
        .unwrap();
    teramind_db::migrate::run(&pool).await.unwrap();
    let repo = teramind_db::repos::StorageStatsRepo::new(pool.clone());
    let raw = tmp.path().join("raw");
    std::fs::create_dir_all(&raw).unwrap();
    teramindd::services::storage_stats::spawn(
        repo.clone(),
        raw,
        "teramind_test".into(),
        std::time::Duration::from_millis(50),
    );
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM storage_stats")
        .fetch_one(pool.pg())
        .await
        .unwrap();
    assert!(n >= 1);
    sup.shutdown().await.unwrap();
}
