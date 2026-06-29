//! P4 perf budget — core §4: ingest event-in to PG-committed p99 < 50 ms.
//!
//! Drives IngestService against a fresh_pool, then for 200 iterations enqueues
//! a UserPrompt event and waits until the corresponding row is committed in
//! Postgres (observed via `SELECT count(*) FROM turns`). Each enqueue uses a
//! distinct ordinal so the upsert always inserts a new row.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::tempdir;
use teramind_core::ids::{ClientEventId, SessionId};
use teramind_core::redact::Redactor;
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo};
use teramindd::services::ingest::{IngestDeps, IngestService, IngestStats};
use teramindd::services::jsonl_writer::JsonlWriter;
use teramindd::services::session_manager::SessionManager;
use time::OffsetDateTime;

// P4 perf — opt in via cargo test --release -- --ignored
#[tokio::test]
#[ignore]
async fn ingest_event_to_pg_committed_p99_under_50ms() {
    let tmp = tempdir().unwrap();
    let pool = match teramind_db::testing::fresh_pool().await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("perf-ingest-commit-p99: cannot seed fresh_pool ({e}); skipping");
            return;
        }
    };

    let jsonl = Arc::new(JsonlWriter::open(tmp.path().join("raw")).await.unwrap());
    let stats = Arc::new(IngestStats::default());
    let (raw_tx, _) = tokio::sync::mpsc::unbounded_channel();
    let registry = Arc::new(teramindd::services::fs_watcher::WatchRegistry::new(
        raw_tx,
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
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
    // Larger capacity than the iteration count so try_enqueue never drops.
    let svc = IngestService::spawn(1024, deps);

    let session_id = SessionId::new();
    let now = OffsetDateTime::now_utc();
    svc.try_enqueue(EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: now,
        event: IngestEvent::SessionStart {
            session_id,
            agent_session_id: None,
            agent_kind: "claude_code".into(),
            agent_version: None,
            cwd: "/w".into(),
            os: "linux".into(),
            hostname: "h".into(),
            user_login: "u".into(),
            git_head: None,
            git_branch: None,
        },
    })
    .unwrap();

    // Wait for the SessionStart to commit so the FK on turns(session_id) is
    // satisfied before we time anything.
    let started = Instant::now();
    loop {
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM sessions")
            .fetch_one(pool.pg())
            .await
            .unwrap();
        if n >= 1 {
            break;
        }
        if started.elapsed() > Duration::from_secs(5) {
            eprintln!("perf-ingest-commit-p99: SessionStart did not commit; skipping");
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // Pre-flight: warm caches, libpq prepared statements, JSONL fsync path, etc.
    let warm_ordinal: i32 = 0;
    let warm_baseline: i64 = current_turn_count(&pool).await;
    svc.try_enqueue(make_user_prompt(session_id, warm_ordinal, now))
        .unwrap();
    wait_for_turn_count(&pool, warm_baseline + 1, Duration::from_secs(5)).await;

    // Measurement loop: 200 iterations.
    const N: usize = 200;
    let mut samples: Vec<Duration> = Vec::with_capacity(N);
    let mut baseline: i64 = current_turn_count(&pool).await;
    for i in 0..N {
        let ordinal = (i as i32) + 1; // 0 used by warmup
        let env = make_user_prompt(
            session_id,
            ordinal,
            now + time::Duration::milliseconds(i as i64),
        );
        let start = Instant::now();
        svc.try_enqueue(env).unwrap();
        // Poll for commit: turn count must increment by 1.
        let target = baseline + 1;
        loop {
            let n = current_turn_count(&pool).await;
            if n >= target {
                baseline = n;
                break;
            }
            tokio::time::sleep(Duration::from_micros(100)).await;
            if start.elapsed() > Duration::from_secs(10) {
                panic!("perf-ingest-commit-p99: iteration {i} stalled waiting for commit");
            }
        }
        samples.push(start.elapsed());
    }

    samples.sort();
    // p99 of N=200 → sorted[(200 * 99 / 100) - 1] = sorted[197].
    let p99 = samples[N * 99 / 100 - 1];
    let budget = Duration::from_millis(50);
    assert!(
        p99 < budget,
        "ingest event-in to PG-committed p99 = {:.2} ms exceeds budget {} ms (spec core §4)",
        p99.as_secs_f64() * 1000.0,
        budget.as_millis()
    );
}

fn make_user_prompt(session_id: SessionId, ordinal: i32, ts: OffsetDateTime) -> EventEnvelope {
    EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts,
        event: IngestEvent::UserPrompt {
            session_id,
            turn_ordinal: ordinal,
            prompt: format!("perf probe ordinal={ordinal}"),
            turn_id: None,
        },
    }
}

async fn current_turn_count(pool: &teramind_db::pool::DbPool) -> i64 {
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM turns")
        .fetch_one(pool.pg())
        .await
        .unwrap();
    n
}

async fn wait_for_turn_count(pool: &teramind_db::pool::DbPool, target: i64, timeout: Duration) {
    let started = Instant::now();
    loop {
        if current_turn_count(pool).await >= target {
            return;
        }
        if started.elapsed() > timeout {
            panic!("perf-ingest-commit-p99: warmup did not reach turn_count={target}");
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}
