//! Perf budget — codifier §10: the problem_fix detector must hit p99 < 1s on
//! the same 1k-session corpus shape (5 turns × 3 tool_calls per turn).
//!
//! Half the sessions carry an "Error: ..." pattern in both the user_prompt
//! (which is what the detector actually inspects via looks_like_error) and
//! in tool_call output (per the dataset directive). Each turn has an
//! associated file_diff so the detector's LEFT-JOIN-via-subquery yields a
//! non-null `diff_agg` and the signature path runs end-to-end.
//!
//! Marked #[ignore] — runs only in the perf sweep. Skipped (eprintln + early
//! return) if seeding fails.

use std::time::{Duration, Instant};
use teramind_core::ids::TurnId;
use teramind_db::repos::diff::NewFileDiff;
use teramind_db::repos::session::NewSession;
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, SkillObservationRepo, TraceRepo};
use teramindd::services::codify::detectors::problem_fix;
use time::OffsetDateTime;
use uuid::Uuid;

const SESSIONS: usize = 1_000;
const TURNS_PER_SESSION: usize = 5;
const TOOLS_PER_TURN: usize = 3;
const ITERATIONS: usize = 50;
const BUDGET_MS: u128 = 1_000;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn perf_problem_fix_detector_p99_under_1s() {
    let pool = match teramind_db::testing::fresh_pool().await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("perf_detector_problem_fix: skipping — fresh_pool unavailable: {e}");
            return;
        }
    };

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let diffs = DiffRepo::new(pool.clone());

    let agent = match agents.upsert("claude_code", None).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("perf_detector_problem_fix: skipping — agent upsert failed: {e}");
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
                    "perf_detector_problem_fix: skipping — session insert {s_idx} failed: {e}"
                );
                return;
            }
        };

        // Half the sessions carry an error pattern in user_prompt + tool_call
        // output; the other half are vanilla so the detector has to scan and
        // skip them (which exercises looks_like_error on every row).
        let is_error_session = s_idx % 2 == 0;
        let prompt: &str = if is_error_session {
            "Error: cannot find type Foo in scope at file.rs:42"
        } else {
            "please refactor this"
        };
        let tool_output: &str = if is_error_session {
            "Error: build failed"
        } else {
            "ok"
        };

        for t in 0..TURNS_PER_SESSION {
            let tid = TurnId(Uuid::new_v4());
            if let Err(e) = trace
                .upsert_turn_with_id(tid, sid, t as i32, started, Some(prompt))
                .await
            {
                eprintln!("perf_detector_problem_fix: skipping — upsert_turn failed: {e}");
                return;
            }
            if let Err(e) = trace
                .finalize_turn(
                    tid,
                    started,
                    Some("Fixed."),
                    None,
                    Some("claude"),
                    None,
                    None,
                )
                .await
            {
                eprintln!("perf_detector_problem_fix: skipping — finalize_turn failed: {e}");
                return;
            }

            // Three tool calls per turn — one carrying the "Error: ..."
            // output (per the directive); detector reads tool output only
            // indirectly via diff_agg, but the directive shape is honoured.
            for c in 0..TOOLS_PER_TURN {
                let ordinal = (t * TOOLS_PER_TURN + c) as i32;
                let tcid = match trace
                    .insert_tool_call_start(
                        tid,
                        ordinal,
                        "Bash",
                        &serde_json::json!({"command": "cargo build"}),
                        started,
                    )
                    .await
                {
                    Ok(id) => id,
                    Err(e) => {
                        eprintln!(
                            "perf_detector_problem_fix: skipping — tool_call insert failed: {e}"
                        );
                        return;
                    }
                };
                let out = if c == 0 { tool_output } else { "ok" };
                let is_err = c == 0 && is_error_session;
                if let Err(e) = trace.finalize_tool_call(tcid, out, is_err, 10).await {
                    eprintln!(
                        "perf_detector_problem_fix: skipping — finalize_tool_call failed: {e}"
                    );
                    return;
                }
            }

            // One file_diff per turn so the detector's diff_agg subquery
            // returns non-null and the signature path executes.
            if let Err(e) = diffs
                .insert(NewFileDiff {
                    turn_id: Some(tid),
                    session_id: sid,
                    file_path: "src/lib.rs",
                    rel_path: "src/lib.rs",
                    attribution: teramind_core::types::file_diff::Attribution::Agent,
                    language: Some("rust"),
                    pre_excerpt: "old",
                    post_excerpt: "new",
                    unified_diff: "- pub fn foo() {}\n+ pub fn foo(x: i32) {}\n",
                    pre_hash: [0u8; 32],
                    post_hash: [1u8; 32],
                    byte_size: 100,
                    captured_at: started,
                })
                .await
            {
                eprintln!("perf_detector_problem_fix: skipping — diff insert failed: {e}");
                return;
            }
        }
    }

    let obs = SkillObservationRepo::new(pool.clone());

    // Warm-up — prime PG prepared-statement cache + connection pool so we
    // measure steady-state, not the cold first call.
    if let Err(e) = problem_fix::run(&pool, &obs, time::Duration::days(30), None).await {
        eprintln!("perf_detector_problem_fix: skipping — warm-up run failed: {e}");
        return;
    }

    let mut samples: Vec<Duration> = Vec::with_capacity(ITERATIONS);
    for i in 0..ITERATIONS {
        let t0 = Instant::now();
        if let Err(e) = problem_fix::run(&pool, &obs, time::Duration::days(30), None).await {
            panic!("perf_detector_problem_fix: detector failed on iter {i}: {e}");
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
        "problem_fix detector p99 regression: observed {p99_ms}ms (budget {BUDGET_MS}ms) \
         over {ITERATIONS} runs on {SESSIONS}-session corpus"
    );
}
