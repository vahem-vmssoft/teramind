//! Perf budget — summarizer §10: digest::build on a 10k-turn session with a
//! 32k char budget must hit p99 < 50ms.
//!
//! Seeds a fresh pool with one session of 10,000 finalized turns (each ~200
//! char user_prompt + assistant_text), loads it via WikiRepo::load_snapshot,
//! then runs digest::build 100 times and asserts p99 < 50ms.
//!
//! Marked #[ignore] — runs only in the perf sweep, not the normal cargo test
//! pass. Skipped (eprintln + early return) if the dataset cannot be seeded.

use std::time::{Duration, Instant};
use teramind_core::ids::TurnId;
use teramind_db::repos::session::NewSession;
use teramind_db::repos::{AgentRepo, SessionRepo, TraceRepo, WikiRepo};
use teramindd::services::summarize::digest;
use time::OffsetDateTime;
use uuid::Uuid;

const TURNS: usize = 10_000;
const ITERATIONS: usize = 100;
const CHAR_BUDGET: usize = 32_000;
const BUDGET_MS: u128 = 50;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn perf_digest_build_p99_under_50ms() {
    let pool = match teramind_db::testing::fresh_pool().await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("perf_digest_build: skipping — fresh_pool unavailable: {e}");
            return;
        }
    };

    // Seed: one session, 10k finalized turns with ~200 char prompt + response.
    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());

    let agent = match agents.upsert("claude_code", None).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("perf_digest_build: skipping — agent upsert failed: {e}");
            return;
        }
    };
    let started = OffsetDateTime::now_utc();
    let sid = match sessions
        .insert(NewSession {
            agent_id: agent.id,
            agent_session_id: None,
            cwd: "/proj",
            project_id: None,
            parent_session_id: None,
            git_head: Some("abc1234567890"),
            git_branch: Some("main"),
            os: "linux",
            hostname: "h",
            user_login: "u",
            started_at: started,
            user_id: None,
            device_id: None,
        })
        .await
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("perf_digest_build: skipping — session insert failed: {e}");
            return;
        }
    };

    // Each turn: ~200-char user_prompt + ~200-char assistant_text. Repeat the
    // ordinal in the text so prompts are distinct and don't all dedupe to one
    // length for the "key prompts" ranking.
    let prompt_base: String = "p".repeat(200);
    let resp_base: String = "r".repeat(200);
    for i in 0..TURNS {
        let tid = TurnId(Uuid::new_v4());
        let prompt = format!("{prompt_base}-{i}");
        let resp = format!("{resp_base}-{i}");
        if let Err(e) = trace
            .upsert_turn_with_id(tid, sid, i as i32, started, Some(&prompt))
            .await
        {
            eprintln!("perf_digest_build: skipping — turn insert failed at {i}: {e}");
            return;
        }
        if let Err(e) = trace
            .finalize_turn(tid, started, Some(&resp), None, Some("claude"), None, None)
            .await
        {
            eprintln!("perf_digest_build: skipping — finalize_turn failed at {i}: {e}");
            return;
        }
    }
    if let Err(e) = sessions
        .end(sid, started + time::Duration::seconds(2000), "stop_hook")
        .await
    {
        eprintln!("perf_digest_build: skipping — session end failed: {e}");
        return;
    }

    let wiki = WikiRepo::new(pool.clone());
    let snapshot = match wiki.load_snapshot(sid).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            eprintln!("perf_digest_build: skipping — load_snapshot returned None");
            return;
        }
        Err(e) => {
            eprintln!("perf_digest_build: skipping — load_snapshot failed: {e}");
            return;
        }
    };
    assert_eq!(
        snapshot.turns.len(),
        TURNS,
        "seed mismatch: expected {TURNS} turns, got {}",
        snapshot.turns.len()
    );

    // Warm-up — first call may pay allocator / branch-predictor costs that
    // skew the p99 unrealistically. Discard one untimed run.
    let _ = digest::build(&snapshot, CHAR_BUDGET);

    let mut samples: Vec<Duration> = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let t0 = Instant::now();
        let out = digest::build(&snapshot, CHAR_BUDGET);
        samples.push(t0.elapsed());
        // Use `out` so the optimiser can't elide the call.
        assert!(out.len() <= CHAR_BUDGET);
    }

    samples.sort();
    // p99 of N=100 -> sorted[98] (0-indexed = N * 99 / 100 - 1).
    let p99_idx = ITERATIONS * 99 / 100 - 1;
    let p99 = samples[p99_idx];
    let p99_ms = p99.as_millis();
    assert!(
        p99_ms < BUDGET_MS,
        "digest::build p99 regression: observed {p99_ms}ms (budget {BUDGET_MS}ms) \
         over {ITERATIONS} runs on {TURNS}-turn snapshot with {CHAR_BUDGET}-char budget"
    );
}
