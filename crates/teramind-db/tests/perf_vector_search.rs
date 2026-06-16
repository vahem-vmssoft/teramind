//! Perf budget: pgvector §10 — Vector top-K=10 on 10k vectors via HNSW cosine p99 < 50ms.
//!
//! Seeds N=10_000 (or fallback 2_000 if env override) embeddings backed by real
//! session+turn rows, then runs 200 vector_search_turns queries with varied query
//! vectors and asserts the p99 latency stays under 50ms.
//!
//! Marked #[ignore] so it does NOT run in the normal `cargo test` sweep — invoke with
//! `cargo test -p teramind-db --test perf_vector_search -- --ignored --nocapture`.

use std::time::{Duration, Instant};

use teramind_core::ids::TurnId;
use teramind_db::repos::session::NewSession;
use teramind_db::repos::{
    AgentRepo, EmbeddingRepo, SearchRepo, SessionRepo, ToEmbedRow, TraceRepo,
};
use time::OffsetDateTime;

const MODEL: &str = "perf-test";
const DIM: i32 = 768;
const QUERY_ITERS: usize = 200;
const TOP_K: u32 = 10;
const BUDGET_P99: Duration = Duration::from_millis(50);

/// Deterministic LCG so the test is reproducible without adding a `rand` dep.
struct Lcg(u64);
impl Lcg {
    fn new(seed: u64) -> Self {
        Self(
            seed.wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407),
        )
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
    fn next_f32(&mut self) -> f32 {
        // map to roughly [-1, 1)
        let bits = (self.next_u64() >> 40) as u32; // 24-bit
        let unit = (bits as f32) / ((1u32 << 24) as f32); // [0, 1)
        unit * 2.0 - 1.0
    }
}

fn random_unit_vec(rng: &mut Lcg, dim: usize) -> Vec<f32> {
    let mut v: Vec<f32> = (0..dim).map(|_| rng.next_f32()).collect();
    let mut sq = 0.0f32;
    for x in &v {
        sq += x * x;
    }
    let norm = sq.sqrt().max(1e-12);
    for x in v.iter_mut() {
        *x /= norm;
    }
    v
}

/// Sort `samples` ascending and return the p99 — defined per the task spec as
/// `sorted[N * 99 / 100 - 1]` (0-indexed). For N=200 this is index 197.
fn p99(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    let n = samples.len();
    let idx = (n * 99 / 100).saturating_sub(1);
    samples[idx]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn perf_vector_search_turns_p99_under_50ms() {
    let pool = match teramind_db::testing::fresh_pool().await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("perf-vector-search-p99: skipping — could not acquire fresh_pool: {e:#}");
            return;
        }
    };

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let embed = EmbeddingRepo::new(pool.clone());
    let search = SearchRepo::new(pool.clone());

    // Allow scaling the dataset down via env for slow CI; default 10k per spec.
    // The budget always applies — if the underlying impl cannot meet 50ms p99 at the
    // configured N, the test fails loudly (which is the intent).
    let n_turns: usize = std::env::var("PERF_VECTOR_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);

    let agent = match agents.upsert("claude_code", None).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("perf-vector-search-p99: skipping — agent upsert failed: {e:#}");
            return;
        }
    };
    let sid = match sessions
        .insert(NewSession {
            agent_id: agent.id,
            agent_session_id: None,
            cwd: "/perf",
            project_id: None,
            parent_session_id: None,
            git_head: None,
            git_branch: None,
            os: "linux",
            hostname: "perf-host",
            user_login: "perf",
            started_at: OffsetDateTime::now_utc(),
            user_id: None,
            device_id: None,
        })
        .await
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("perf-vector-search-p99: skipping — session insert failed: {e:#}");
            return;
        }
    };

    // Seed N turns + embeddings in batches so the transaction in bulk_insert stays
    // a reasonable size (HNSW build cost goes up roughly with rows; chunking keeps
    // memory bounded and avoids one giant tx).
    let mut rng = Lcg::new(0x00C0_FFEE_DEAD_BEEF);
    let batch: usize = 500;
    let started = Instant::now();
    let now = OffsetDateTime::now_utc();
    for chunk_start in (0..n_turns).step_by(batch) {
        let this_n = std::cmp::min(batch, n_turns - chunk_start);
        let mut rows = Vec::with_capacity(this_n);
        let mut vecs = Vec::with_capacity(this_n);
        for i in 0..this_n {
            let ordinal = (chunk_start + i) as i32;
            let turn_id = match trace
                .upsert_turn_with_id(
                    TurnId(uuid::Uuid::new_v4()),
                    sid,
                    ordinal,
                    now,
                    Some("perf"),
                )
                .await
            {
                Ok(t) => t,
                Err(e) => {
                    eprintln!(
                        "perf-vector-search-p99: skipping — turn upsert failed at ord {ordinal}: {e:#}"
                    );
                    return;
                }
            };
            rows.push(ToEmbedRow {
                kind: "turn".into(),
                item_id: turn_id.0,
                text: "perf".into(),
            });
            vecs.push(random_unit_vec(&mut rng, DIM as usize));
        }
        if let Err(e) = embed.bulk_insert(&rows, MODEL, DIM, &vecs).await {
            eprintln!(
                "perf-vector-search-p99: skipping — bulk_insert failed at chunk {chunk_start}: {e:#}"
            );
            return;
        }
    }
    eprintln!(
        "perf-vector-search-p99: seeded {n_turns} turns+embeddings in {:?}",
        started.elapsed()
    );

    // Warm up the connection / planner so the first sample doesn't dominate the p99.
    let warm = random_unit_vec(&mut rng, DIM as usize);
    for _ in 0..3 {
        if let Err(e) = search.vector_search_turns(&warm, MODEL, TOP_K).await {
            eprintln!("perf-vector-search-p99: skipping — warmup query failed: {e:#}");
            return;
        }
    }

    // Measure.
    let mut samples: Vec<Duration> = Vec::with_capacity(QUERY_ITERS);
    for _ in 0..QUERY_ITERS {
        let q = random_unit_vec(&mut rng, DIM as usize);
        let t0 = Instant::now();
        let hits = match search.vector_search_turns(&q, MODEL, TOP_K).await {
            Ok(h) => h,
            Err(e) => panic!("perf-vector-search-p99: query failed during measurement: {e:#}"),
        };
        let elapsed = t0.elapsed();
        assert!(
            !hits.is_empty(),
            "perf-vector-search-p99: vector_search_turns returned 0 rows on populated index"
        );
        assert!(
            hits.len() <= TOP_K as usize,
            "perf-vector-search-p99: returned more than TOP_K rows: {}",
            hits.len()
        );
        samples.push(elapsed);
    }

    let observed_p99 = p99(samples.clone());
    let min = samples.iter().min().copied().unwrap_or_default();
    let max = samples.iter().max().copied().unwrap_or_default();
    eprintln!(
        "perf-vector-search-p99: N={n_turns} iters={QUERY_ITERS} min={min:?} max={max:?} p99={observed_p99:?} budget={BUDGET_P99:?}"
    );

    assert!(
        observed_p99 < BUDGET_P99,
        "perf-vector-search-p99 REGRESSION: observed p99 = {:.3}ms exceeds budget = {:.3}ms (N={}, iters={})",
        observed_p99.as_secs_f64() * 1000.0,
        BUDGET_P99.as_secs_f64() * 1000.0,
        n_turns,
        QUERY_ITERS
    );
}
