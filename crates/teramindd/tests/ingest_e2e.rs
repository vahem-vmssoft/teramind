use std::sync::Arc;
use tempfile::tempdir;
use teramind_core::ids::{ClientEventId, SessionId};
use teramind_core::redact::Redactor;
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo};
use teramindd::services::ingest::{IngestDeps, IngestService, IngestStats};
use teramindd::services::jsonl_writer::JsonlWriter;
use teramindd::services::session_manager::SessionManager;
use time::OffsetDateTime;

#[tokio::test]
async fn ingest_session_start_then_user_prompt_writes_rows() {
    let tmp = tempdir().unwrap();
    let pool = teramind_db::testing::fresh_pool().await.unwrap();

    let jsonl = Arc::new(JsonlWriter::open(tmp.path().join("raw")).await.unwrap());
    let stats = Arc::new(IngestStats::default());
    let (raw_tx, _) = tokio::sync::mpsc::unbounded_channel();
    let registry = std::sync::Arc::new(teramindd::services::fs_watcher::WatchRegistry::new(
        raw_tx,
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
    ));
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
        write_tool_ring: teramindd::services::write_tool_ring::WriteToolRing::new(
            64,
            time::Duration::seconds(5),
        ),
        fs_registry: registry,
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
}

#[tokio::test]
async fn jsonl_shadow_log_is_redacted() {
    // Regression: the JSONL shadow log feeds both `grep_fallback` search
    // results and the team-sync forwarder, so writing the raw envelope here
    // would leak secrets through both paths. Redaction must run before the
    // JSONL append.
    let tmp = tempdir().unwrap();
    let pool = teramind_db::testing::fresh_pool().await.unwrap();

    let raw_dir = tmp.path().join("raw");
    let jsonl = Arc::new(JsonlWriter::open(raw_dir.clone()).await.unwrap());
    let stats = Arc::new(IngestStats::default());
    let (raw_tx, _) = tokio::sync::mpsc::unbounded_channel();
    let registry = std::sync::Arc::new(teramindd::services::fs_watcher::WatchRegistry::new(
        raw_tx,
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
    ));
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
        write_tool_ring: teramindd::services::write_tool_ring::WriteToolRing::new(
            64,
            time::Duration::seconds(5),
        ),
        fs_registry: registry,
    };
    let svc = IngestService::spawn(64, deps);

    let session_id = SessionId::new();
    let now = OffsetDateTime::now_utc();
    svc.try_enqueue(EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: now,
        event: IngestEvent::SessionStart {
            session_id,
            agent_session_id: None,
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
            prompt: "leak key=AKIAIOSFODNN7EXAMPLE here".into(),
            turn_id: None,
        },
    })
    .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // The JSONL writer rotates daily; read every *.jsonl file under raw_dir.
    let mut bodies = String::new();
    for entry in std::fs::read_dir(&raw_dir).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            bodies.push_str(&std::fs::read_to_string(&p).unwrap());
        }
    }
    assert!(
        !bodies.is_empty(),
        "expected at least one JSONL file under {raw_dir:?}"
    );
    assert!(
        !bodies.contains("AKIAIOSFODNN7EXAMPLE"),
        "raw secret leaked to JSONL shadow log:\n{bodies}"
    );
}

#[tokio::test]
async fn storage_stats_sampler_records_a_row() {
    let tmp = tempfile::tempdir().unwrap();
    let pool = teramind_db::testing::fresh_pool().await.unwrap();
    let repo = teramind_db::repos::StorageStatsRepo::new(pool.clone());
    let raw = tmp.path().join("raw");
    std::fs::create_dir_all(&raw).unwrap();
    // The test fixture gives each test a uniquely-named database, so the
    // sampler must use that actual name — passing a literal would make
    // pg_database_size() error out and no row would be inserted.
    let (db_name,): (String,) = sqlx::query_as("SELECT current_database()")
        .fetch_one(pool.pg())
        .await
        .unwrap();
    teramindd::services::storage_stats::spawn(
        repo.clone(),
        raw,
        db_name,
        std::time::Duration::from_millis(50),
    );
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM storage_stats")
        .fetch_one(pool.pg())
        .await
        .unwrap();
    assert!(n >= 1);
}
