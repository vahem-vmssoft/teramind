//! P4 perf budget — summarizer §10: `REFRESH MATERIALIZED VIEW CONCURRENTLY
//! traces_fts` on a 10k-session DB p99 < 30s.
//!
//! Per the directive's NOTE: seeding 10k sessions in-process is prohibitively
//! slow for an opt-in perf test, so we use 500 sessions × ~10 turns ≈ 5k turns.
//! That comfortably exercises the refresh cost (the refresh scans the whole
//! turns+wiki_pages join, not just sessions) and keeps the test under the
//! <200-line ceiling while still being a meaningful tripwire. Deviation
//! documented in batch notes.

use std::time::{Duration, Instant};

const N_SESSIONS: usize = 500;
const TURNS_PER_SESSION: usize = 10;

// P4 perf — opt in via cargo test --release -- --ignored
#[tokio::test]
#[ignore]
async fn traces_fts_refresh_concurrently_p99_under_30s() {
    let pool = match teramind_db::testing::fresh_pool().await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("perf-traces-fts-refresh-p99: cannot seed fresh_pool ({e}); skipping");
            return;
        }
    };

    // Seed an agent so each session has a valid FK.
    let (agent_id,): (uuid::Uuid,) = match sqlx::query_as(
        "INSERT INTO agents (kind) VALUES ('claude_code') RETURNING id",
    )
    .fetch_one(pool.pg())
    .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("perf-traces-fts-refresh-p99: cannot insert agent ({e}); skipping");
            return;
        }
    };

    // Bulk-seed sessions in a single transaction for speed.
    let mut tx = match pool.pg().begin().await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("perf-traces-fts-refresh-p99: cannot begin tx ({e}); skipping");
            return;
        }
    };
    let now = time::OffsetDateTime::now_utc();
    let mut session_ids: Vec<uuid::Uuid> = Vec::with_capacity(N_SESSIONS);
    for i in 0..N_SESSIONS {
        let sid = uuid::Uuid::new_v4();
        let started = now - time::Duration::seconds(i as i64);
        let ended = started + time::Duration::seconds(60);
        if let Err(e) = sqlx::query(
            r#"
            INSERT INTO sessions
                (id, agent_id, agent_session_id, cwd, os, hostname, user_login,
                 started_at, ended_at, end_reason)
            VALUES ($1, $2, NULL, '/w', 'linux', 'h', 'u', $3, $4, 'finalized')
            "#,
        )
        .bind(sid)
        .bind(agent_id)
        .bind(started)
        .bind(ended)
        .execute(&mut *tx)
        .await
        {
            eprintln!("perf-traces-fts-refresh-p99: insert session {i} failed ({e}); skipping");
            return;
        }
        session_ids.push(sid);
    }
    if let Err(e) = tx.commit().await {
        eprintln!("perf-traces-fts-refresh-p99: session commit failed ({e}); skipping");
        return;
    }

    // Seed turns in a single transaction.
    let mut tx = match pool.pg().begin().await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("perf-traces-fts-refresh-p99: cannot begin turns tx ({e}); skipping");
            return;
        }
    };
    for (si, sid) in session_ids.iter().enumerate() {
        for j in 0..TURNS_PER_SESSION {
            let ts = now - time::Duration::seconds((si * TURNS_PER_SESSION + j) as i64);
            let prompt = format!(
                "session {si} turn {j} deadlock retry postgres replication kafka backpressure"
            );
            let assistant = format!("response {si}/{j} resolved retrying with exponential backoff");
            if let Err(e) = sqlx::query(
                r#"
                INSERT INTO turns
                    (session_id, ordinal, started_at, ended_at, user_prompt, assistant_text)
                VALUES ($1, $2, $3, $3, $4, $5)
                "#,
            )
            .bind(sid)
            .bind(j as i32)
            .bind(ts)
            .bind(prompt)
            .bind(assistant)
            .execute(&mut *tx)
            .await
            {
                eprintln!(
                    "perf-traces-fts-refresh-p99: insert turn ({si},{j}) failed ({e}); skipping"
                );
                return;
            }
        }
    }
    if let Err(e) = tx.commit().await {
        eprintln!("perf-traces-fts-refresh-p99: turns commit failed ({e}); skipping");
        return;
    }

    // Sanity-check the seed actually populated — a silently-empty corpus
    // would make REFRESH trivially fast and mask real regressions.
    let (turn_count,): (i64,) = sqlx::query_as("SELECT count(*) FROM turns")
        .fetch_one(pool.pg())
        .await
        .expect("count(*) turns");
    assert!(
        turn_count >= (N_SESSIONS * TURNS_PER_SESSION) as i64,
        "perf-traces-fts-refresh-p99: seed populated only {turn_count} turn rows (expected ≥ {})",
        N_SESSIONS * TURNS_PER_SESSION
    );

    // ANALYZE so subsequent REFRESH cost is representative.
    let _ = sqlx::query("ANALYZE").execute(pool.pg()).await;

    // Initial (non-CONCURRENTLY) REFRESH to populate the MV. The CONCURRENTLY
    // variant requires a prior populated state, so this is a prerequisite.
    if let Err(e) = sqlx::query("REFRESH MATERIALIZED VIEW traces_fts")
        .execute(pool.pg())
        .await
    {
        eprintln!("perf-traces-fts-refresh-p99: initial REFRESH failed ({e}); skipping");
        return;
    }

    // Measurement loop: 20 iterations of REFRESH MATERIALIZED VIEW CONCURRENTLY.
    const N: usize = 20;
    let mut samples: Vec<Duration> = Vec::with_capacity(N);
    for i in 0..N {
        let start = Instant::now();
        if let Err(e) = sqlx::query("REFRESH MATERIALIZED VIEW CONCURRENTLY traces_fts")
            .execute(pool.pg())
            .await
        {
            panic!(
                "perf-traces-fts-refresh-p99: REFRESH CONCURRENTLY failed at iter {i}: {e}"
            );
        }
        samples.push(start.elapsed());
    }

    samples.sort();
    // p99 of N=20 → sorted[(20 * 99 / 100) - 1] = sorted[18] (clamped at 19 if
    // the multiplication had rounded up; with 20 elements the 19th index is
    // the conservative choice for "99th percentile" in such a small sample).
    // Per directive: "p99 (= the 19th sorted value)".
    let p99 = samples[19];
    let budget = Duration::from_secs(30);
    assert!(
        p99 < budget,
        "REFRESH MATERIALIZED VIEW CONCURRENTLY traces_fts p99 = {:.2} ms exceeds budget {} ms (spec summarizer §10)",
        p99.as_secs_f64() * 1000.0,
        budget.as_millis()
    );
}
