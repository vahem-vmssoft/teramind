//! P4 perf budget — pgvector §10: blended search with semantic_weight=0.4
//! p95 ≤ 1s (target ≤ 800ms).
//!
//! Seeds ~500 turns + corresponding embeddings (random-ish 768-d vectors so
//! the vector index has to do real work), refreshes traces_fts, then runs
//! 100 iterations of do_search and asserts the p95 budget.

use async_trait::async_trait;
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use std::sync::Arc;
use std::time::{Duration, Instant};
use teramind_core::embed::{DistanceMetric, EmbedError, EmbeddingProvider, ProviderKind};
use teramind_core::types::SearchRequest;
use teramind_db::repos::{
    AgentRepo, EmbeddingRepo, SearchRepo, SessionRepo, ToEmbedRow, TraceRepo,
};
use teramindd::services::search::{self, BlendWeights};

const MODEL: &str = "perf:mock-768";
const DIM: usize = 768;
const N_TURNS: usize = 500;

struct MockRandomProvider {
    rng_seed: u64,
}

#[async_trait]
impl EmbeddingProvider for MockRandomProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Fastembed
    }
    fn model_id(&self) -> &str {
        MODEL
    }
    fn dimension(&self) -> usize {
        DIM
    }
    fn max_tokens(&self) -> usize {
        8192
    }
    fn distance_metric(&self) -> DistanceMetric {
        DistanceMetric::Cosine
    }
    async fn health_check(&self) -> Result<(), EmbedError> {
        Ok(())
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        // Deterministic per-query: hash the input length + seed into the RNG.
        let mut rng = StdRng::seed_from_u64(self.rng_seed.wrapping_add(texts.len() as u64));
        Ok(texts.iter().map(|_| random_unit_vec(&mut rng)).collect())
    }
}

fn random_unit_vec(rng: &mut StdRng) -> Vec<f32> {
    let mut v: Vec<f32> = (0..DIM).map(|_| rng.random_range(-1.0..1.0_f32)).collect();
    // L2-normalize so cosine distance is meaningful.
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    for x in &mut v {
        *x /= norm;
    }
    v
}

// P4 perf — opt in via cargo test --release -- --ignored
#[tokio::test]
#[ignore]
async fn blended_search_p95_under_1s() {
    let pool = match teramind_db::testing::fresh_pool().await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("perf-search-blended-p95: cannot seed fresh_pool ({e}); skipping");
            return;
        }
    };

    // Seed one agent + one session to hang turns off of.
    let agents = AgentRepo::new(pool.clone());
    let agent = match agents.upsert("claude_code", None).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("perf-search-blended-p95: cannot upsert agent ({e}); skipping");
            return;
        }
    };
    let sessions = SessionRepo::new(pool.clone());
    let now = time::OffsetDateTime::now_utc();
    let sid = match sessions
        .insert(teramind_db::repos::session::NewSession {
            agent_id: agent.id,
            agent_session_id: None,
            cwd: "/w",
            project_id: None,
            parent_session_id: None,
            git_head: None,
            git_branch: None,
            os: "linux",
            hostname: "h",
            user_login: "u",
            started_at: now,
            user_id: None,
            device_id: None,
        })
        .await
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("perf-search-blended-p95: cannot insert session ({e}); skipping");
            return;
        }
    };

    let trace = TraceRepo::new(pool.clone());
    let embed_repo = EmbeddingRepo::new(pool.clone());
    let mut rng = StdRng::seed_from_u64(0xC0FFEE);

    let mut to_embed = Vec::with_capacity(N_TURNS);
    let mut vectors = Vec::with_capacity(N_TURNS);
    for i in 0..N_TURNS {
        let ts = now + time::Duration::seconds(i as i64);
        // Vary prompt text so FTS has differentiated tokens.
        let prompt = format!(
            "iteration {i} deadlock retry kafka consumer queue replication lag postgres index"
        );
        let turn_id = match trace.upsert_turn(sid, i as i32, ts, Some(&prompt)).await {
            Ok(t) => t,
            Err(e) => {
                eprintln!("perf-search-blended-p95: upsert_turn failed at i={i} ({e}); skipping");
                return;
            }
        };
        let assistant = format!("response {i} resolved by retrying with backoff");
        if let Err(e) = trace
            .finalize_turn(turn_id, ts, Some(&assistant), None, None, None, None)
            .await
        {
            eprintln!("perf-search-blended-p95: finalize_turn failed at i={i} ({e}); skipping");
            return;
        }
        to_embed.push(ToEmbedRow {
            kind: "turn".into(),
            item_id: turn_id.0,
            text: prompt,
        });
        vectors.push(random_unit_vec(&mut rng));
    }

    if let Err(e) = embed_repo
        .bulk_insert(&to_embed, MODEL, DIM as i32, &vectors)
        .await
    {
        eprintln!("perf-search-blended-p95: embeddings bulk_insert failed ({e}); skipping");
        return;
    }

    if let Err(e) = sqlx::query("REFRESH MATERIALIZED VIEW traces_fts")
        .execute(pool.pg())
        .await
    {
        eprintln!("perf-search-blended-p95: REFRESH traces_fts failed ({e}); skipping");
        return;
    }

    // ANALYZE so the planner picks reasonable plans for the seeded data.
    let _ = sqlx::query("ANALYZE").execute(pool.pg()).await;

    let provider: Arc<dyn EmbeddingProvider> = Arc::new(MockRandomProvider {
        rng_seed: 0xBADC0DE,
    });
    let repo = SearchRepo::new(pool.clone());
    let weights = BlendWeights {
        fts: 0.6,
        trgm: 0.4,
        semantic: 0.4,
        recency: 0.2,
        project: 0.3,
    };
    let req = SearchRequest {
        query: "deadlock retry".into(),
        limit: 10,
    };

    // Warmup — first calls pay the cost of plan caching + connection acquisition.
    for _ in 0..3 {
        let _ = search::do_search(&repo, Some(provider.clone()), MODEL, weights, &req).await;
    }

    const N: usize = 100;
    let mut samples: Vec<Duration> = Vec::with_capacity(N);
    for _ in 0..N {
        let start = Instant::now();
        let out = search::do_search(&repo, Some(provider.clone()), MODEL, weights, &req)
            .await
            .expect("do_search failed");
        samples.push(start.elapsed());
        // Sanity: with semantic=0.4 the provider returned vectors → not degraded.
        assert!(
            !out.degraded,
            "search degraded despite provider returning vectors"
        );
    }

    samples.sort();
    // p95 of N=100 → sorted[(100 * 95 / 100) - 1] = sorted[94].
    let p95 = samples[N * 95 / 100 - 1];
    let budget = Duration::from_millis(1000);
    assert!(
        p95 < budget,
        "blended search (semantic_weight=0.4) p95 = {:.2} ms exceeds budget {} ms (spec pgvector §10)",
        p95.as_secs_f64() * 1000.0,
        budget.as_millis()
    );
}
