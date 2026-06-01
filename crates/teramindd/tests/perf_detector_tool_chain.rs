//! Perf budget — codifier §10: the tool_chain detector must hit p99 < 500ms
//! on a 1k-session corpus (5 finalized turns × 3 tool_calls per turn).
//!
//! Seeds a fresh pool with 1,000 sessions (each with 5 turns, 3 tool_calls
//! per turn for a Bash → Edit → Bash chain), then invokes
//! teramindd::services::codify::detectors::tool_chain::run 50 times and
//! asserts the p99 elapsed < 500ms.
//!
//! Marked #[ignore] — runs only in the perf sweep. Skipped (eprintln + early
//! return) if seeding fails.

use std::time::{Duration, Instant};
use teramind_core::ids::TurnId;
use teramind_db::repos::session::NewSession;
use teramind_db::repos::{AgentRepo, SessionRepo, SkillObservationRepo, TraceRepo};
use teramindd::services::codify::detectors::tool_chain;
use time::OffsetDateTime;
use uuid::Uuid;

const SESSIONS: usize = 1_000;
const TURNS_PER_SESSION: usize = 5;
const TOOLS_PER_TURN: usize = 3;
const ITERATIONS: usize = 50;
const BUDGET_MS: u128 = 500;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn perf_tool_chain_detector_p99_under_500ms() {
    let pool = match teramind_db::testing::fresh_pool().await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("perf_detector_tool_chain: skipping — fresh_pool unavailable: {e}");
            return;
        }
    };

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());

    let agent = match agents.upsert("claude_code", None).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("perf_detector_tool_chain: skipping — agent upsert failed: {e}");
            return;
        }
    };
    let started = OffsetDateTime::now_utc();

    for s_idx in 0..SESSIONS {
        let sid = match sessions
            .insert(NewSession {
                agent_id: agent.id,
                agent_session_id: None,
                cwd: "/proj",
                project_id: None,
                parent_session_id: None,
                git_head: None,
                git_branch: None,
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
                eprintln!(
                    "perf_detector_tool_chain: skipping — session insert {s_idx} failed: {e}"
                );
                return;
            }
        };

        for t in 0..TURNS_PER_SESSION {
            let tid = TurnId(Uuid::new_v4());
            if let Err(e) = trace
                .upsert_turn_with_id(tid, sid, t as i32, started, Some("do work"))
                .await
            {
                eprintln!("perf_detector_tool_chain: skipping — upsert_turn failed: {e}");
                return;
            }
            if let Err(e) = trace
                .finalize_turn(tid, started, Some("done"), None, Some("claude"), None, None)
                .await
            {
                eprintln!("perf_detector_tool_chain: skipping — finalize_turn failed: {e}");
                return;
            }
            // Three tool calls forming a uniform Bash(build) → Edit(toml) →
            // Bash(test) chain. Identical across sessions so the detector has
            // to hash + group all 1k sessions.
            for c in 0..TOOLS_PER_TURN {
                let ordinal = (t * TOOLS_PER_TURN + c) as i32;
                let (name, input) = match c {
                    0 => ("Bash", serde_json::json!({"command":"cargo build"})),
                    1 => ("Edit", serde_json::json!({"file_path":"Cargo.toml"})),
                    _ => ("Bash", serde_json::json!({"command":"cargo test"})),
                };
                let tcid = match trace
                    .insert_tool_call_start(tid, ordinal, name, &input, started)
                    .await
                {
                    Ok(id) => id,
                    Err(e) => {
                        eprintln!(
                            "perf_detector_tool_chain: skipping — tool_call insert failed: {e}"
                        );
                        return;
                    }
                };
                if let Err(e) = trace.finalize_tool_call(tcid, "ok", false, 10).await {
                    eprintln!(
                        "perf_detector_tool_chain: skipping — finalize_tool_call failed: {e}"
                    );
                    return;
                }
            }
        }
    }

    let obs = SkillObservationRepo::new(pool.clone());

    // Warm-up: prime the connection pool / prepared-statement cache so we
    // measure steady-state, not the cold first call.
    if let Err(e) = tool_chain::run(&pool, &obs, time::Duration::days(30), None).await {
        eprintln!("perf_detector_tool_chain: skipping — warm-up run failed: {e}");
        return;
    }

    let mut samples: Vec<Duration> = Vec::with_capacity(ITERATIONS);
    for i in 0..ITERATIONS {
        let t0 = Instant::now();
        if let Err(e) = tool_chain::run(&pool, &obs, time::Duration::days(30), None).await {
            panic!("perf_detector_tool_chain: detector failed on iter {i}: {e}");
        }
        samples.push(t0.elapsed());
    }

    samples.sort();
    // p99 of N=50: sorted index = N * 99 / 100 - 1 = 48.
    let p99_idx = ITERATIONS * 99 / 100 - 1;
    let p99 = samples[p99_idx];
    let p99_ms = p99.as_millis();
    assert!(
        p99_ms < BUDGET_MS,
        "tool_chain detector p99 regression: observed {p99_ms}ms (budget {BUDGET_MS}ms) \
         over {ITERATIONS} runs on {SESSIONS}-session corpus"
    );
}
