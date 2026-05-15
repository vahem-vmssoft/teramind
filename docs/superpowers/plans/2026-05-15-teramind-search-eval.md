# Teramind Search Effectiveness Benchmark — Plan F

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an L5 search-effectiveness benchmark that scores Teramind's ranking against a labelled query/relevance corpus, writes a Markdown scorecard + `eval-results.json`, compares against a committed baseline, and fails CI when ranking quality regresses past documented thresholds.

**Architecture:** A new bin crate `teramind-search-eval` ships three subcommands: `generate-corpus` (deterministic synthetic corpus to JSONL + `qrels.toml`), `run` (load corpus into a throwaway embedded Postgres, execute every query in `queries.toml` through `SearchRepo::fts_turns` + `trgm_diffs` + the `rank_and_hydrate` blend, score with nDCG@10/MRR/P@K/R@K), and `compare-baseline` (read `baseline.json`, exit non-zero on regression). A GitHub Actions workflow gates PRs touching `crates/teramind-db/src/repos/search.rs` or `crates/teramindd/src/services/search.rs`. `teramind doctor` learns to print the local-corpus nDCG@10 once the baseline exists.

**Tech Stack:** Rust stable, existing workspace (sqlx, tokio, postgresql_embedded, serde, serde_json, toml), pure-math metric module with proptest, deterministic `rand_chacha::ChaCha20Rng` for reproducible corpus.

---

## Scope check

Spec §9.5 defines this single benchmark. Within scope:
- Corpus generator + committed corpus (sub-scaled to 500 sessions for git practicality; `--scale=2000` flag matches spec target at run-time).
- 100 hand-curated queries across 5 intent classes (≥20 each).
- nDCG@10 (headline), MRR, P@5, P@10, R@10 — per class and overall.
- Regression gates per spec table.
- CI gate on PRs that touch search paths.
- `teramind doctor` reports local nDCG@10.

Deferred: real-user corpus contribution (spec calls this post-v1).

---

## File structure

**New files (all under `crates/teramind-search-eval/` unless noted):**

| File | Responsibility |
|---|---|
| `crates/teramind-search-eval/Cargo.toml` | bin crate manifest |
| `crates/teramind-search-eval/src/main.rs` | clap dispatcher (`generate-corpus` / `run` / `compare-baseline`) |
| `crates/teramind-search-eval/src/lib.rs` | re-exports module tree |
| `crates/teramind-search-eval/src/metrics.rs` | pure math: `ndcg_at_k`, `mrr`, `precision_at_k`, `recall_at_k` |
| `crates/teramind-search-eval/src/types.rs` | `Query`, `QueryClass`, `Qrels`, `RankedHit`, `EvalReport`, `Baseline` |
| `crates/teramind-search-eval/src/corpus.rs` | JSONL load + DB ingest |
| `crates/teramind-search-eval/src/generator.rs` | deterministic corpus + qrels generator |
| `crates/teramind-search-eval/src/queries_bank.rs` | hand-curated 100-query bank |
| `crates/teramind-search-eval/src/harness.rs` | spin up PG, refresh `traces_fts`, run queries |
| `crates/teramind-search-eval/src/reporter.rs` | aggregate metrics, write JSON + Markdown |
| `crates/teramind-search-eval/src/gates.rs` | regression gate logic |
| `benches/search-eval/queries.toml` | 100 hand-curated queries (generator-emitted) |
| `benches/search-eval/corpus/*.jsonl` | generated corpus (committed, ~2 MB total) |
| `benches/search-eval/qrels.toml` | generated relevance judgments (committed) |
| `benches/search-eval/baseline.json` | locked baseline metrics (committed) |
| `benches/search-eval/README.md` | corpus authoring guide |
| `.github/workflows/search-eval.yml` | CI gate |

**Modified files:**
- `Cargo.toml` (workspace) — add `teramind-search-eval` to members; add `rand`, `rand_chacha`, `toml` to workspace deps.
- `crates/teramind/src/commands/doctor.rs` — append local-corpus nDCG@10 if `benches/search-eval/baseline.json` exists.
- `crates/teramind/Cargo.toml` — depend on `teramind-search-eval` (for the `Baseline` type).
- `.gitignore` — ignore `benches/search-eval/eval-results.json` and `eval-scorecard.md`.

---

## Section 0 — Workspace setup

### Task 0.1: Add `teramind-search-eval` crate scaffold

**Files:**
- Modify: `Cargo.toml` (workspace)
- Create: `crates/teramind-search-eval/Cargo.toml`
- Create: `crates/teramind-search-eval/src/main.rs`
- Create: `crates/teramind-search-eval/src/lib.rs`
- Create: `crates/teramind-search-eval/src/{metrics,types,corpus,generator,harness,reporter,gates,queries_bank}.rs` (stubs)

- [ ] **Step 1: Add workspace dep + member**

In the root `Cargo.toml`, add to `[workspace] members`:

```toml
    "crates/teramind-search-eval",
```

(Insert in the existing alphabetical position; before `"crates/teramindd"`.)

Append to `[workspace.dependencies]` (alphabetical):

```toml
rand        = "0.8"
rand_chacha = "0.3"
```

(`toml` is already used by `teramindd` directly with `toml = "0.8"`; we add it to the workspace later in this task.)

Add `toml = "0.8"` to `[workspace.dependencies]` as well, and update `crates/teramindd/Cargo.toml` to use `toml = { workspace = true }` instead of its direct version.

- [ ] **Step 2: Scaffold the crate manifest**

Create `crates/teramind-search-eval/Cargo.toml`:

```toml
[package]
name = "teramind-search-eval"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[[bin]]
name = "teramind-search-eval"
path = "src/main.rs"

[lib]
name = "teramind_search_eval"
path = "src/lib.rs"

[dependencies]
teramind-core = { path = "../teramind-core" }
teramind-db   = { path = "../teramind-db" }
teramindd     = { path = "../teramindd" }
anyhow      = { workspace = true }
clap        = { workspace = true }
serde       = { workspace = true }
serde_json  = { workspace = true }
toml        = { workspace = true }
tokio       = { workspace = true }
tracing     = { workspace = true }
tracing-subscriber = { workspace = true }
sqlx        = { workspace = true }
time        = { workspace = true }
uuid        = { workspace = true }
rand        = { workspace = true }
rand_chacha = { workspace = true }
sha2        = { workspace = true }
hex         = { workspace = true }

[dev-dependencies]
proptest = { workspace = true }
tempfile = { workspace = true }
```

- [ ] **Step 3: Stub `main.rs` + `lib.rs`**

Create `crates/teramind-search-eval/src/lib.rs`:

```rust
//! Library half of `teramind-search-eval`. The CLI binary in `main.rs`
//! is a thin shell around these modules so they remain testable.
pub mod metrics;
pub mod types;
pub mod corpus;
pub mod generator;
pub mod queries_bank;
pub mod harness;
pub mod reporter;
pub mod gates;
```

Create `crates/teramind-search-eval/src/main.rs`:

```rust
use clap::{Parser, Subcommand};

/// Teramind search-effectiveness benchmark.
#[derive(Debug, Parser)]
#[command(name = "teramind-search-eval", about = "Run the L5 search benchmark.")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Regenerate `benches/search-eval/corpus/*.jsonl` and `qrels.toml`
    /// from the deterministic seed.
    GenerateCorpus {
        /// Number of synthetic sessions to emit. Default 500.
        #[arg(long, default_value = "500")]
        scale: u32,
        /// Output root. Defaults to `benches/search-eval/`.
        #[arg(long)]
        out: Option<std::path::PathBuf>,
    },
    /// Load the corpus into a throwaway DB, run every query, write
    /// `eval-results.json` + a Markdown scorecard.
    Run {
        /// Path to the corpus root.
        #[arg(long, default_value = "benches/search-eval")]
        corpus: std::path::PathBuf,
        /// Output directory for `eval-results.json` and the scorecard.
        #[arg(long, default_value = "benches/search-eval")]
        out: std::path::PathBuf,
    },
    /// Compare `eval-results.json` against `baseline.json` and exit
    /// non-zero if any regression gate trips.
    CompareBaseline {
        #[arg(long, default_value = "benches/search-eval/eval-results.json")]
        results: std::path::PathBuf,
        #[arg(long, default_value = "benches/search-eval/baseline.json")]
        baseline: std::path::PathBuf,
        /// Rewrite baseline.json from results (use only with [eval-baseline-update]).
        #[arg(long)]
        update_baseline: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::try_init().ok();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::GenerateCorpus { scale, out } => {
            let dest = out.unwrap_or_else(|| "benches/search-eval".into());
            teramind_search_eval::generator::generate_to(&dest, scale)
        }
        Cmd::Run { corpus, out } => {
            teramind_search_eval::harness::run(&corpus, &out).await
        }
        Cmd::CompareBaseline { results, baseline, update_baseline } => {
            teramind_search_eval::gates::compare(&results, &baseline, update_baseline)
        }
    }
}
```

- [ ] **Step 4: Stub each module so the crate builds**

Create the following stub files. Each contains only a doc-comment so the crate compiles. Real bodies arrive in later sections.

Create `crates/teramind-search-eval/src/metrics.rs`:

```rust
//! Ranking-metric primitives. See Section 1.
```

Create `crates/teramind-search-eval/src/types.rs`:

```rust
//! Query/qrels/report types. See Section 2.
```

Create `crates/teramind-search-eval/src/corpus.rs`:

```rust
//! Corpus JSONL load + DB ingest. See Section 3.
```

Create `crates/teramind-search-eval/src/generator.rs`:

```rust
//! Deterministic synthetic corpus generator. See Section 4.

use std::path::Path;

pub fn generate_to(_dest: &Path, _scale: u32) -> anyhow::Result<()> {
    anyhow::bail!("not implemented yet (Section 4 of Plan F)")
}
```

Create `crates/teramind-search-eval/src/queries_bank.rs`:

```rust
//! Hand-curated query bank. Populated in Section 5.

use crate::types::QueryClass;

pub struct QueryBankEntry {
    pub id: &'static str,
    pub class: QueryClass,
    pub text: &'static str,
    pub triggers: &'static [&'static str],
}

// Placeholder so the crate compiles; full bank arrives in Section 5.
pub const QUERIES: &[QueryBankEntry] = &[];
```

Create `crates/teramind-search-eval/src/harness.rs`:

```rust
//! Eval harness: spin up PG, run queries. See Section 6.

use std::path::Path;

pub async fn run(_corpus: &Path, _out: &Path) -> anyhow::Result<()> {
    anyhow::bail!("not implemented yet (Section 6 of Plan F)")
}
```

Create `crates/teramind-search-eval/src/reporter.rs`:

```rust
//! Metric aggregation + JSON / Markdown emission. See Section 7.
```

Create `crates/teramind-search-eval/src/gates.rs`:

```rust
//! Regression-gate comparison. See Section 8.

use std::path::Path;

pub fn compare(_results: &Path, _baseline: &Path, _update: bool) -> anyhow::Result<()> {
    anyhow::bail!("not implemented yet (Section 8 of Plan F)")
}
```

- [ ] **Step 5: Verify the crate builds**

Run: `cargo check -p teramind-search-eval`
Expected: succeeds (warnings about unused stubs are fine).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/teramind-search-eval/ crates/teramindd/Cargo.toml
git commit -m "feat(search-eval): scaffold teramind-search-eval crate"
```

---

## Section 1 — Metric helpers

Pure math; no I/O; ideal for TDD + proptest.

### Task 1.1: `ndcg_at_k`

**Files:**
- Modify: `crates/teramind-search-eval/src/metrics.rs`

- [ ] **Step 1: Write the failing tests + scaffold**

Replace `crates/teramind-search-eval/src/metrics.rs` with:

```rust
//! Ranking-metric primitives.
//!
//! All functions take `relevance: &[u32]` where each entry is the graded
//! relevance (0 = irrelevant, 1 = relevant, 2 = strongly relevant) of
//! the i-th *ranked* hit. Index 0 is the top hit.

/// Discounted cumulative gain at rank `k`.
///
/// DCG@K = sum (rel_i / log2(i + 2)) for i in 0..min(K, ranked.len())
pub fn dcg_at_k(relevance: &[u32], k: usize) -> f64 {
    relevance
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, &r)| (r as f64) / ((i as f64 + 2.0).log2()))
        .sum()
}

/// Ideal DCG@K — DCG of the ranking sorted descending by relevance.
pub fn idcg_at_k(relevance: &[u32], k: usize) -> f64 {
    let mut sorted: Vec<u32> = relevance.to_vec();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    dcg_at_k(&sorted, k)
}

/// Normalized DCG@K: nDCG = DCG / IDCG.
///
/// Returns 0.0 when there are no relevant items (IDCG = 0).
pub fn ndcg_at_k(relevance: &[u32], k: usize) -> f64 {
    let i = idcg_at_k(relevance, k);
    if i == 0.0 { return 0.0; }
    (dcg_at_k(relevance, k) / i).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dcg_known_answer() {
        // Relevance [3, 2, 3, 0, 1, 2] -> DCG@6 ~ 6.86 (classic textbook example).
        let r: Vec<u32> = vec![3, 2, 3, 0, 1, 2];
        let dcg = dcg_at_k(&r, 6);
        assert!((dcg - 6.8611).abs() < 0.001, "got {dcg}");
    }

    #[test]
    fn ndcg_of_ideal_ranking_is_one() {
        let r: Vec<u32> = vec![2, 2, 1, 1, 0];
        assert!((ndcg_at_k(&r, 5) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn ndcg_of_reversed_ranking_is_less_than_one() {
        let r: Vec<u32> = vec![0, 1, 1, 2, 2];
        assert!(ndcg_at_k(&r, 5) < 1.0);
    }

    #[test]
    fn ndcg_zero_when_no_relevant_hits() {
        let r: Vec<u32> = vec![0, 0, 0];
        assert_eq!(ndcg_at_k(&r, 3), 0.0);
    }

    #[test]
    fn ndcg_handles_short_lists() {
        let r: Vec<u32> = vec![2];
        assert!((ndcg_at_k(&r, 10) - 1.0).abs() < 1e-9);
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramind-search-eval metrics`
Expected: 5 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-search-eval/src/metrics.rs
git commit -m "feat(search-eval): nDCG@K with dcg/idcg helpers"
```

---

### Task 1.2: `mrr` + `precision_at_k` + `recall_at_k`

**Files:**
- Modify: `crates/teramind-search-eval/src/metrics.rs`

- [ ] **Step 1: Append the failing tests**

Append to the `tests` module:

```rust
    #[test]
    fn mrr_takes_reciprocal_rank_of_first_relevant() {
        // Relevance > 0 starts at index 2 (rank 3) -> MRR = 1/3.
        let r: Vec<u32> = vec![0, 0, 2, 1];
        assert!((mrr(&r) - (1.0 / 3.0)).abs() < 1e-9);
    }

    #[test]
    fn mrr_zero_when_no_relevant() {
        let r: Vec<u32> = vec![0, 0, 0];
        assert_eq!(mrr(&r), 0.0);
    }

    #[test]
    fn precision_at_k_counts_relevant_in_top_k() {
        let r: Vec<u32> = vec![1, 0, 2, 0, 1];
        assert!((precision_at_k(&r, 5) - 0.6).abs() < 1e-9);
        assert!((precision_at_k(&r, 3) - (2.0 / 3.0)).abs() < 1e-9);
    }

    #[test]
    fn precision_at_k_short_list_uses_actual_len() {
        // When k > len, the denominator is still k -> matches IR convention.
        let r: Vec<u32> = vec![1, 1];
        assert!((precision_at_k(&r, 5) - 0.4).abs() < 1e-9);
    }

    #[test]
    fn recall_at_k_uses_total_relevant_count() {
        // total_relevant = 3, top-K hits = 2 relevant -> 2/3.
        let r: Vec<u32> = vec![1, 0, 1, 0];
        assert!((recall_at_k(&r, 4, 3) - (2.0 / 3.0)).abs() < 1e-9);
    }

    #[test]
    fn recall_at_k_zero_when_no_relevant_in_corpus() {
        let r: Vec<u32> = vec![0, 0];
        assert_eq!(recall_at_k(&r, 2, 0), 0.0);
    }

    use proptest::prelude::*;
    proptest! {
        #[test]
        fn ndcg_always_between_zero_and_one(
            rels in proptest::collection::vec(0u32..=2u32, 0..50),
            k    in 1usize..50usize,
        ) {
            let n = ndcg_at_k(&rels, k);
            prop_assert!(n >= 0.0 && n <= 1.0, "nDCG out of [0,1]: {n}");
        }
    }
```

- [ ] **Step 2: Implement the helpers**

Append to `metrics.rs` (above the `tests` module):

```rust
/// Mean Reciprocal Rank for a single ranked list: 1 / rank of the first
/// hit with relevance > 0. Returns 0.0 when no relevant hit is found.
pub fn mrr(relevance: &[u32]) -> f64 {
    relevance
        .iter()
        .enumerate()
        .find(|(_, &r)| r > 0)
        .map(|(i, _)| 1.0 / (i as f64 + 1.0))
        .unwrap_or(0.0)
}

/// Precision@K: fraction of the top-K hits that are relevant.
/// Denominator is `k` (not `min(k, len)`) — matches standard IR convention.
pub fn precision_at_k(relevance: &[u32], k: usize) -> f64 {
    if k == 0 { return 0.0; }
    let hit = relevance.iter().take(k).filter(|&&r| r > 0).count();
    hit as f64 / k as f64
}

/// Recall@K: fraction of all relevant items in the corpus that appear
/// in the top-K hits. Returns 0.0 when `total_relevant == 0`.
pub fn recall_at_k(relevance: &[u32], k: usize, total_relevant: u32) -> f64 {
    if total_relevant == 0 { return 0.0; }
    let hit = relevance.iter().take(k).filter(|&&r| r > 0).count();
    hit as f64 / total_relevant as f64
}
```

- [ ] **Step 3: Run all metrics tests**

Run: `cargo test -p teramind-search-eval metrics`
Expected: PASS (5 + 6 unit tests + 1 proptest = 12 tests).

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-search-eval/src/metrics.rs
git commit -m "feat(search-eval): mrr + precision_at_k + recall_at_k with proptest"
```

---

## Section 2 — Eval types

### Task 2.1: Query, Qrels, EvalReport, Baseline

**Files:**
- Modify: `crates/teramind-search-eval/src/types.rs`

- [ ] **Step 1: Write the failing roundtrip test**

Replace `crates/teramind-search-eval/src/types.rs` with:

```rust
//! Query/qrels/report types shared by the generator, harness, reporter,
//! and gate modules.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryClass {
    NaturalLanguage,
    StackTrace,
    CodeSnippet,
    ToolTyped,
    SymbolicPath,
}

impl QueryClass {
    pub fn all() -> &'static [QueryClass] {
        use QueryClass::*;
        &[NaturalLanguage, StackTrace, CodeSnippet, ToolTyped, SymbolicPath]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Query {
    pub id: String,
    pub class: QueryClass,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QueriesFile {
    pub queries: Vec<Query>,
}

/// `qrels.toml` shape: per-query, a list of (item_id, grade) tuples.
/// Item id encodes the hit kind: "turn:<uuid>", "tool:<uuid>", "diff:<uuid>", "skill:<uuid>".
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QrelsFile {
    pub judgments: BTreeMap<String, Vec<Judgment>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Judgment {
    pub item: String,
    pub grade: u32,
}

/// One row of metrics — either a per-class slice or the overall row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricsRow {
    pub n_queries: usize,
    pub ndcg_at_10: f64,
    pub mrr: f64,
    pub p_at_5: f64,
    pub p_at_10: f64,
    pub r_at_10: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvalReport {
    pub overall: MetricsRow,
    pub by_class: BTreeMap<QueryClass, MetricsRow>,
    /// p95 latency per single-query execution (milliseconds).
    pub query_latency_p95_ms: u32,
    pub corpus_size: CorpusSize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CorpusSize {
    pub sessions: u32,
    pub turns: u32,
    pub tool_calls: u32,
    pub file_diffs: u32,
}

/// `baseline.json` is structurally identical to `EvalReport`.
pub type Baseline = EvalReport;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_file_roundtrips_through_toml() {
        let qf = QueriesFile {
            queries: vec![Query {
                id: "nl-1".into(),
                class: QueryClass::NaturalLanguage,
                text: "how did we fix the JWT bug".into(),
            }],
        };
        let s = toml::to_string(&qf).unwrap();
        let back: QueriesFile = toml::from_str(&s).unwrap();
        assert_eq!(back.queries.len(), 1);
        assert_eq!(back.queries[0].id, "nl-1");
        assert!(matches!(back.queries[0].class, QueryClass::NaturalLanguage));
    }

    #[test]
    fn qrels_file_roundtrips_through_toml() {
        let mut judgments = BTreeMap::new();
        judgments.insert(
            "nl-1".into(),
            vec![Judgment { item: "turn:abc".into(), grade: 2 }],
        );
        let qrels = QrelsFile { judgments };
        let s = toml::to_string(&qrels).unwrap();
        let back: QrelsFile = toml::from_str(&s).unwrap();
        assert_eq!(back.judgments.get("nl-1").unwrap()[0].grade, 2);
    }

    #[test]
    fn eval_report_roundtrips_through_json() {
        let mut by_class = BTreeMap::new();
        by_class.insert(QueryClass::NaturalLanguage, MetricsRow {
            n_queries: 20, ndcg_at_10: 0.8, mrr: 0.7, p_at_5: 0.6, p_at_10: 0.5, r_at_10: 0.4,
        });
        let report = EvalReport {
            overall: MetricsRow {
                n_queries: 100, ndcg_at_10: 0.75, mrr: 0.6, p_at_5: 0.55, p_at_10: 0.5, r_at_10: 0.45,
            },
            by_class,
            query_latency_p95_ms: 250,
            corpus_size: CorpusSize { sessions: 500, turns: 2500, tool_calls: 5000, file_diffs: 500 },
        };
        let j = serde_json::to_string(&report).unwrap();
        let back: EvalReport = serde_json::from_str(&j).unwrap();
        assert_eq!(report, back);
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramind-search-eval types`
Expected: 3 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-search-eval/src/types.rs
git commit -m "feat(search-eval): Query/Qrels/EvalReport types"
```

---

## Section 3 — Corpus loader + ingest

### Task 3.1: Corpus row types + JSONL loader

**Files:**
- Modify: `crates/teramind-search-eval/src/corpus.rs`

- [ ] **Step 1: Write the failing test**

Replace `crates/teramind-search-eval/src/corpus.rs` with:

```rust
//! Corpus JSONL loader + DB ingest.

use serde::{Deserialize, Serialize};
use std::io::BufRead;
use std::path::Path;
use teramind_core::types::file_diff::Attribution;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRow {
    pub id: Uuid,
    pub agent_kind: String,
    pub cwd: String,
    pub project_tag: String,    // for grouping; not in DB schema
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRow {
    pub id: Uuid,
    pub session_id: Uuid,
    pub ordinal: i32,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    pub user_prompt: Option<String>,
    pub assistant_text: Option<String>,
    pub thinking: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRow {
    pub id: Uuid,
    pub turn_id: Uuid,
    pub ordinal: i32,
    pub name: String,
    pub input: serde_json::Value,
    pub output: String,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiffRow {
    pub id: Uuid,
    pub session_id: Uuid,
    pub turn_id: Option<Uuid>,
    pub file_path: String,
    pub rel_path: String,
    pub attribution: Attribution,
    pub language: Option<String>,
    pub pre_excerpt: String,
    pub post_excerpt: String,
    pub unified_diff: String,
    #[serde(with = "time::serde::rfc3339")]
    pub captured_at: OffsetDateTime,
}

#[derive(Debug, Default)]
pub struct Corpus {
    pub sessions: Vec<SessionRow>,
    pub turns: Vec<TurnRow>,
    pub tool_calls: Vec<ToolCallRow>,
    pub file_diffs: Vec<FileDiffRow>,
}

pub fn load(root: &Path) -> anyhow::Result<Corpus> {
    let dir = root.join("corpus");
    Ok(Corpus {
        sessions:   load_jsonl(&dir.join("sessions.jsonl"))?,
        turns:      load_jsonl(&dir.join("turns.jsonl"))?,
        tool_calls: load_jsonl(&dir.join("tool_calls.jsonl"))?,
        file_diffs: load_jsonl(&dir.join("file_diffs.jsonl"))?,
    })
}

fn load_jsonl<T: serde::de::DeserializeOwned>(path: &Path) -> anyhow::Result<Vec<T>> {
    if !path.exists() { return Ok(Vec::new()); }
    let f = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(f);
    let mut out = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() { continue; }
        out.push(serde_json::from_str::<T>(&line)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_session_row() -> SessionRow {
        SessionRow {
            id: Uuid::nil(),
            agent_kind: "claude_code".into(),
            cwd: "/tmp/proj".into(),
            project_tag: "rust-web".into(),
            started_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        }
    }

    #[test]
    fn jsonl_roundtrips_session_rows() {
        let dir = TempDir::new().unwrap();
        let corpus_dir = dir.path().join("corpus");
        std::fs::create_dir_all(&corpus_dir).unwrap();
        let row = sample_session_row();
        let line = serde_json::to_string(&row).unwrap();
        std::fs::write(corpus_dir.join("sessions.jsonl"), line).unwrap();
        let c = load(dir.path()).unwrap();
        assert_eq!(c.sessions.len(), 1);
        assert_eq!(c.sessions[0].project_tag, "rust-web");
    }

    #[test]
    fn missing_files_yield_empty_corpus() {
        let dir = TempDir::new().unwrap();
        let c = load(dir.path()).unwrap();
        assert!(c.sessions.is_empty());
        assert!(c.turns.is_empty());
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramind-search-eval corpus`
Expected: 2 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-search-eval/src/corpus.rs
git commit -m "feat(search-eval): corpus row types + JSONL loader"
```

---

### Task 3.2: Corpus → DB ingest

**Files:**
- Modify: `crates/teramind-search-eval/src/corpus.rs`

- [ ] **Step 1: Append the ingest function**

Append to `corpus.rs` (above the `tests` module):

```rust
use teramind_core::ids::{FileDiffId, SessionId, ToolCallId, TurnId};
use teramind_db::pool::DbPool;
use teramind_db::repos::diff::NewFileDiff;
use teramind_db::repos::session::NewSession;
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo};

pub async fn ingest(pool: &DbPool, c: &Corpus) -> anyhow::Result<()> {
    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let diffs = DiffRepo::new(pool.clone());

    let claude_agent = agents.upsert("claude_code", None).await?;

    for s in &c.sessions {
        let _ = sessions.insert_with_id(SessionId(s.id), NewSession {
            agent_id: claude_agent.id,
            agent_session_id: None,
            cwd: &s.cwd,
            project_id: None,
            parent_session_id: None,
            git_head: None,
            git_branch: None,
            os: "linux",
            hostname: "eval",
            user_login: "eval",
            started_at: s.started_at,
        }).await?;
    }
    for t in &c.turns {
        let tid = trace.upsert_turn_with_id(
            TurnId(t.id),
            SessionId(t.session_id),
            t.ordinal,
            t.started_at,
            t.user_prompt.as_deref(),
        ).await?;
        trace.finalize_turn(
            tid, t.started_at,
            t.assistant_text.as_deref(),
            t.thinking.as_deref(),
            Some("eval-model"), None, None,
        ).await?;
    }
    for tc in &c.tool_calls {
        let _ = trace.insert_tool_call_start_with_id(
            ToolCallId(tc.id),
            TurnId(tc.turn_id),
            tc.ordinal, &tc.name, &tc.input, tc.started_at,
        ).await?;
        trace.finalize_tool_call(ToolCallId(tc.id), &tc.output, false, 0).await?;
    }
    for d in &c.file_diffs {
        let _: FileDiffId = diffs.insert(NewFileDiff {
            turn_id: d.turn_id.map(TurnId),
            session_id: SessionId(d.session_id),
            file_path: &d.file_path,
            rel_path: &d.rel_path,
            attribution: d.attribution,
            language: d.language.as_deref(),
            pre_excerpt: &d.pre_excerpt,
            post_excerpt: &d.post_excerpt,
            unified_diff: &d.unified_diff,
            pre_hash: [0u8; 32],
            post_hash: [1u8; 32],
            byte_size: d.post_excerpt.len() as i32,
            captured_at: d.captured_at,
        }).await?;
    }

    // Refresh the FTS materialized view so search hits anything we just wrote.
    sqlx::query("REFRESH MATERIALIZED VIEW traces_fts")
        .execute(pool.pg()).await?;

    Ok(())
}
```

- [ ] **Step 2: Write the test**

Append (inside the `tests` module) at the bottom of `corpus.rs`:

```rust
    use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ingest_loads_a_minimal_corpus_into_pg() -> anyhow::Result<()> {
        let dir = TempDir::new().unwrap();
        let pgdata = dir.path().join("pgdata");
        let sup = PgSupervisor::start(pgdata, "teramind").await?;
        let pool = DbPool::connect(sup.connect_options()).await?;
        migrate::run(&pool).await?;

        let c = Corpus {
            sessions: vec![SessionRow {
                id: Uuid::new_v4(),
                agent_kind: "claude_code".into(),
                cwd: "/proj".into(),
                project_tag: "rust-web".into(),
                started_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            }],
            turns: vec![], tool_calls: vec![], file_diffs: vec![],
        };
        ingest(&pool, &c).await?;

        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM sessions")
            .fetch_one(pool.pg()).await?;
        assert_eq!(n, 1);

        sup.shutdown().await?;
        Ok(())
    }
```

- [ ] **Step 3: Run**

Run: `cargo test -p teramind-search-eval ingest_loads_a_minimal_corpus_into_pg --release`
Expected: PASS (release build keeps PG startup fast).

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-search-eval/src/corpus.rs
git commit -m "feat(search-eval): Corpus -> DB ingest with FTS refresh"
```

---

## Section 4 — Deterministic corpus generator

### Task 4.1: Generator core: project templates + RNG + trigger injection

**Files:**
- Modify: `crates/teramind-search-eval/src/generator.rs`

- [ ] **Step 1: Write the test + full generator**

Replace `crates/teramind-search-eval/src/generator.rs` with:

```rust
//! Deterministic synthetic corpus + qrels generator.
//!
//! Same `(seed, scale)` always yields the same files. The corpus is
//! sub-scaled by default (500 sessions) to keep the committed JSONL
//! under ~2 MB; pass `--scale=2000` for full spec parity at run-time.

use crate::corpus::{Corpus, FileDiffRow, SessionRow, ToolCallRow, TurnRow};
use crate::queries_bank::QUERIES;
use crate::types::{Judgment, QrelsFile, Query, QueryClass, QueriesFile};
use rand::distributions::WeightedIndex;
use rand::prelude::*;
use rand_chacha::ChaCha20Rng;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use teramind_core::types::file_diff::Attribution;
use time::OffsetDateTime;
use uuid::Uuid;

const SEED: u64 = 0xC0_FFEE_C0_FFEE;

#[derive(Debug, Clone, Copy)]
struct Template {
    tag: &'static str,
    cwd: &'static str,
    seed_tokens: &'static [&'static str],
}

const TEMPLATES: &[Template] = &[
    Template { tag: "rust-web",    cwd: "/proj/rust-web",    seed_tokens: &["axum router", "tower middleware", "serde_json", "tokio spawn"] },
    Template { tag: "python-data", cwd: "/proj/python-data", seed_tokens: &["pandas DataFrame", "scikit-learn", "numpy vectorize", "matplotlib"] },
    Template { tag: "ts-react",    cwd: "/proj/ts-react",    seed_tokens: &["useState hook", "react-query", "tsx Component", "tailwind"] },
    Template { tag: "go-cli",      cwd: "/proj/go-cli",      seed_tokens: &["cobra command", "context.Context", "errgroup.Wait", "flag.StringVar"] },
];

pub fn generate_to(dest: &Path, scale: u32) -> anyhow::Result<()> {
    let mut rng = ChaCha20Rng::seed_from_u64(SEED);
    let (corpus, qrels) = build(&mut rng, scale);
    write_outputs(dest, &corpus, &qrels)?;
    println!(
        "teramind-search-eval: wrote {} sessions / {} turns / {} diffs to {}",
        corpus.sessions.len(),
        corpus.turns.len(),
        corpus.file_diffs.len(),
        dest.display(),
    );
    Ok(())
}

fn build(rng: &mut ChaCha20Rng, scale: u32) -> (Corpus, QrelsFile) {
    let triggers_by_query: Vec<(String, QueryClass, &'static [&'static str])> = QUERIES.iter()
        .map(|q| (q.id.to_string(), q.class, q.triggers))
        .collect();

    let template_weights: Vec<u32> = TEMPLATES.iter().map(|_| 1u32).collect();
    let template_dist = WeightedIndex::new(&template_weights).unwrap();

    let mut corpus = Corpus::default();
    let mut qrels: BTreeMap<String, Vec<Judgment>> = BTreeMap::new();
    let base_ts = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();

    for s_idx in 0..scale {
        let tpl = &TEMPLATES[template_dist.sample(rng)];
        let session_id = deterministic_uuid(rng);
        corpus.sessions.push(SessionRow {
            id: session_id,
            agent_kind: "claude_code".to_string(),
            cwd: tpl.cwd.to_string(),
            project_tag: tpl.tag.to_string(),
            started_at: base_ts + time::Duration::seconds(s_idx as i64 * 60),
        });

        let n_turns: u32 = rng.gen_range(2..=5);
        for t_idx in 0..n_turns {
            let turn_id = deterministic_uuid(rng);
            let chosen_query_idx: Option<usize> = if triggers_by_query.is_empty() {
                None
            } else if rng.gen_bool(0.20) {
                Some(rng.gen_range(0..triggers_by_query.len()))
            } else {
                None
            };

            let mut prompt = format!("{} -- task on {}", pick_one(rng, tpl.seed_tokens), tpl.tag);
            let mut assistant = format!("worked on the {} task", tpl.tag);

            if let Some(qi) = chosen_query_idx {
                let (qid, class, triggers) = &triggers_by_query[qi];
                let trig = pick_one(rng, triggers);
                match class {
                    QueryClass::NaturalLanguage => { prompt.push_str(&format!(" -- {}", trig)); }
                    QueryClass::StackTrace      => { assistant.push_str(&format!("\n{}", trig)); }
                    QueryClass::SymbolicPath    => { assistant.push_str(&format!(" using {}", trig)); }
                    QueryClass::CodeSnippet     => { /* planted in the file_diff below */ }
                    QueryClass::ToolTyped       => { /* planted in the tool_call below */ }
                }
                qrels.entry(qid.clone()).or_default().push(Judgment {
                    item: format!("turn:{}", turn_id),
                    grade: 2,
                });
            }

            corpus.turns.push(TurnRow {
                id: turn_id,
                session_id,
                ordinal: t_idx as i32,
                started_at: base_ts + time::Duration::seconds(s_idx as i64 * 60 + t_idx as i64),
                user_prompt: Some(prompt),
                assistant_text: Some(assistant),
                thinking: None,
            });

            let n_tools: u32 = rng.gen_range(0..=2);
            for tc_idx in 0..n_tools {
                let tool_id = deterministic_uuid(rng);
                let mut tool_output = format!("ran {} successfully", pick_one(rng, &["test", "bench", "build"]));
                let mut tool_name = pick_one(rng, &["Bash", "Read", "Grep"]);

                if let Some(qi) = chosen_query_idx {
                    let (qid, class, triggers) = &triggers_by_query[qi];
                    if matches!(class, QueryClass::ToolTyped) {
                        let trig = pick_one(rng, triggers);
                        tool_output = trig.to_string();
                        tool_name = "Edit";
                        qrels.entry(qid.clone()).or_default().push(Judgment {
                            item: format!("tool:{}", tool_id),
                            grade: 2,
                        });
                    }
                }

                corpus.tool_calls.push(ToolCallRow {
                    id: tool_id,
                    turn_id,
                    ordinal: tc_idx as i32,
                    name: tool_name.to_string(),
                    input: serde_json::json!({"x": tc_idx}),
                    output: tool_output,
                    started_at: base_ts + time::Duration::seconds(s_idx as i64 * 60 + t_idx as i64),
                });
            }

            if rng.gen_bool(0.4) {
                let diff_id = deterministic_uuid(rng);
                let rel_path = format!("src/{}.rs", pick_one(rng, &["lib", "util", "parser"]));
                let mut pre_excerpt = format!("fn old_{} {{}}", s_idx);
                let mut post_excerpt = format!("fn new_{} {{}}", s_idx);

                if let Some(qi) = chosen_query_idx {
                    let (qid, class, triggers) = &triggers_by_query[qi];
                    if matches!(class, QueryClass::CodeSnippet) {
                        let trig = pick_one(rng, triggers);
                        pre_excerpt = format!("{}\n{}", trig, pre_excerpt);
                        post_excerpt = format!("{}\n{}", trig, post_excerpt);
                        qrels.entry(qid.clone()).or_default().push(Judgment {
                            item: format!("diff:{}", diff_id),
                            grade: 2,
                        });
                    }
                }

                corpus.file_diffs.push(FileDiffRow {
                    id: diff_id,
                    session_id,
                    turn_id: Some(turn_id),
                    file_path: format!("{}/{}", tpl.cwd, rel_path),
                    rel_path,
                    attribution: Attribution::Agent,
                    language: Some("rust".into()),
                    pre_excerpt,
                    post_excerpt,
                    unified_diff: "@@ stub @@".into(),
                    captured_at: base_ts + time::Duration::seconds(s_idx as i64 * 60 + t_idx as i64),
                });
            }
        }
    }

    (corpus, QrelsFile { judgments: qrels })
}

fn deterministic_uuid(rng: &mut ChaCha20Rng) -> Uuid {
    let mut bytes = [0u8; 16];
    rng.fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    Uuid::from_bytes(bytes)
}

fn pick_one<'a>(rng: &mut ChaCha20Rng, slice: &'a [&'a str]) -> &'a str {
    slice[rng.gen_range(0..slice.len())]
}

fn write_outputs(dest: &Path, corpus: &Corpus, qrels: &QrelsFile) -> anyhow::Result<()> {
    let corpus_dir = dest.join("corpus");
    std::fs::create_dir_all(&corpus_dir)?;
    write_jsonl(&corpus_dir.join("sessions.jsonl"),   &corpus.sessions)?;
    write_jsonl(&corpus_dir.join("turns.jsonl"),      &corpus.turns)?;
    write_jsonl(&corpus_dir.join("tool_calls.jsonl"), &corpus.tool_calls)?;
    write_jsonl(&corpus_dir.join("file_diffs.jsonl"), &corpus.file_diffs)?;
    std::fs::write(dest.join("qrels.toml"), toml::to_string_pretty(qrels)?)?;
    std::fs::write(dest.join("queries.toml"), build_queries_toml()?)?;
    Ok(())
}

fn write_jsonl<T: Serialize>(path: &Path, rows: &[T]) -> anyhow::Result<()> {
    use std::io::Write;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    for r in rows {
        serde_json::to_writer(&mut f, r)?;
        f.write_all(b"\n")?;
    }
    f.flush()?;
    Ok(())
}

fn build_queries_toml() -> anyhow::Result<String> {
    let queries: Vec<Query> = QUERIES.iter().map(|q| Query {
        id: q.id.into(),
        class: q.class,
        text: q.text.into(),
    }).collect();
    Ok(toml::to_string_pretty(&QueriesFile { queries })?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generator_is_deterministic_for_same_seed() {
        let mut a = ChaCha20Rng::seed_from_u64(SEED);
        let mut b = ChaCha20Rng::seed_from_u64(SEED);
        let (ca, qa) = build(&mut a, 50);
        let (cb, qb) = build(&mut b, 50);
        assert_eq!(ca.sessions.len(), cb.sessions.len());
        assert_eq!(
            ca.sessions.iter().map(|s| s.id).collect::<Vec<_>>(),
            cb.sessions.iter().map(|s| s.id).collect::<Vec<_>>(),
        );
        assert_eq!(qa.judgments.len(), qb.judgments.len());
    }

    #[test]
    fn generator_produces_expected_scale() {
        let mut rng = ChaCha20Rng::seed_from_u64(SEED);
        let (c, _) = build(&mut rng, 100);
        assert_eq!(c.sessions.len(), 100);
        assert!(!c.turns.is_empty());
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramind-search-eval generator`
Expected: 2 tests PASS. (`qrels` will be empty until §5 populates the query bank — the test only asserts `.judgments.len()` matches between runs, which is 0 == 0.)

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-search-eval/src/generator.rs
git commit -m "feat(search-eval): deterministic corpus + qrels generator"
```

---

## Section 5 — Queries bank (100 across 5 classes)

### Task 5.1: Populate the query bank

**Files:**
- Modify: `crates/teramind-search-eval/src/queries_bank.rs`
- Create: `crates/teramind-search-eval/tests/queries_bank.rs`

- [ ] **Step 1: Write the validation tests first**

Create `crates/teramind-search-eval/tests/queries_bank.rs`:

```rust
use teramind_search_eval::queries_bank::QUERIES;
use teramind_search_eval::types::QueryClass;

#[test]
fn at_least_one_hundred_queries() {
    assert!(QUERIES.len() >= 100, "got {}", QUERIES.len());
}

#[test]
fn every_class_has_at_least_twenty_queries() {
    for class in QueryClass::all() {
        let n = QUERIES.iter().filter(|q| q.class == *class).count();
        assert!(n >= 20, "class {:?} only has {} queries", class, n);
    }
}

#[test]
fn ids_are_unique() {
    let mut ids: Vec<&str> = QUERIES.iter().map(|q| q.id).collect();
    ids.sort();
    let before = ids.len();
    ids.dedup();
    assert_eq!(before, ids.len(), "duplicate ids in QUERIES");
}

#[test]
fn every_query_has_at_least_one_trigger() {
    for q in QUERIES {
        assert!(!q.triggers.is_empty(), "query {} has no triggers", q.id);
    }
}
```

- [ ] **Step 2: Run, confirm failure**

Run: `cargo test -p teramind-search-eval --test queries_bank`
Expected: FAIL (placeholder bank has 0 entries).

- [ ] **Step 3: Replace `queries_bank.rs` with 100 entries**

Replace `crates/teramind-search-eval/src/queries_bank.rs` with:

```rust
//! Hand-curated query bank: 100 queries across 5 intent classes (≥20 each).
//!
//! Each entry's `triggers` slice is the corpus generator's contract — the
//! generator plants one of those tokens in any session it tags as
//! "relevant to this query".

use crate::types::QueryClass;

pub struct QueryBankEntry {
    pub id: &'static str,
    pub class: QueryClass,
    pub text: &'static str,
    pub triggers: &'static [&'static str],
}

pub const QUERIES: &[QueryBankEntry] = &[
    // Natural Language (nl-*)
    nl("nl-01", "how did we fix the JWT expiry bug",                       &["JWT expiry fix", "expiry bug"]),
    nl("nl-02", "what changed about the rate limiter last week",           &["rate limiter rewrite", "rate limit fix"]),
    nl("nl-03", "did anyone solve the redis connection pool leak",         &["redis pool leak", "connection pool fix"]),
    nl("nl-04", "explain the migration to tokio 1.x",                      &["tokio upgrade", "tokio 1.0 migration"]),
    nl("nl-05", "where do we set the read replica routing",                &["read replica", "replica routing"]),
    nl("nl-06", "how is the websocket reconnect handled",                  &["websocket reconnect", "ws reconnect backoff"]),
    nl("nl-07", "summary of the auth refactor",                            &["auth refactor", "auth middleware rewrite"]),
    nl("nl-08", "why did we switch from reqwest to ureq",                  &["reqwest to ureq", "switched http client"]),
    nl("nl-09", "what does the worker pool sizing logic do",               &["worker pool sizing", "pool sizing heuristic"]),
    nl("nl-10", "fix the cors preflight failure",                          &["cors preflight", "preflight fix"]),
    nl("nl-11", "why does the smoke test flake on macos",                  &["macos smoke flake", "smoke test flake"]),
    nl("nl-12", "where is the postgres backoff configured",                &["postgres backoff", "pg reconnect backoff"]),
    nl("nl-13", "how do we generate sitemaps",                             &["sitemap generation", "sitemap pipeline"]),
    nl("nl-14", "explain the new pricing column",                          &["pricing column", "price tier column"]),
    nl("nl-15", "what does the cli doctor do",                             &["doctor command", "diagnostic doctor"]),
    nl("nl-16", "trace the audit log entries",                             &["audit log entries", "audit trail"]),
    nl("nl-17", "why is the cache invalidation so slow",                   &["cache invalidation slow", "slow cache flush"]),
    nl("nl-18", "what was the fix for the pagination off-by-one",          &["pagination off by one", "off by one fix"]),
    nl("nl-19", "explain the new error envelope",                          &["error envelope", "structured error envelope"]),
    nl("nl-20", "where are http retries configured",                       &["http retry config", "retry middleware"]),
    // Stack Trace (st-*)
    st("st-01", "thread main panicked at serializer.rs:142",               &["panicked at serializer.rs:142", "serializer panic"]),
    st("st-02", "NullPointerException at UserController.java:88",          &["NullPointerException at UserController", "NPE at UserController"]),
    st("st-03", "TypeError: cannot read properties of undefined",          &["TypeError: cannot read properties", "undefined property error"]),
    st("st-04", "RuntimeError: dictionary changed size during iteration",  &["dictionary changed size during iteration", "dict size change"]),
    st("st-05", "panic: runtime error: index out of range",                &["index out of range", "panic index out of range"]),
    st("st-06", "sqlx error: column already exists",                       &["sqlx column already exists", "column exists error"]),
    st("st-07", "thread tokio-runtime-worker panicked",                    &["tokio-runtime-worker panicked", "runtime worker panic"]),
    st("st-08", "AttributeError: NoneType object has no attribute",        &["NoneType has no attribute", "AttributeError None"]),
    st("st-09", "ECONNREFUSED 127.0.0.1:5432",                             &["ECONNREFUSED 127.0.0.1:5432", "connection refused pg"]),
    st("st-10", "unrecoverable error: heap out of memory",                 &["heap out of memory", "OOM heap"]),
    st("st-11", "ImportError: cannot import name foo",                     &["cannot import name foo", "ImportError foo"]),
    st("st-12", "Exception in thread http-handler",                        &["http-handler exception", "thread http-handler"]),
    st("st-13", "fatal: not a git repository",                             &["not a git repository", "git fatal repo"]),
    st("st-14", "ValueError: too many values to unpack",                   &["too many values to unpack", "ValueError unpack"]),
    st("st-15", "unhandled rejection: socket hang up",                     &["socket hang up", "unhandled rejection socket"]),
    st("st-16", "panic: assignment to entry in nil map",                   &["assignment to entry in nil map", "nil map go"]),
    st("st-17", "OperationalError: server closed the connection",          &["server closed the connection", "OperationalError"]),
    st("st-18", "stack overflow",                                          &["stack overflow", "max call stack"]),
    st("st-19", "Permission denied publickey",                             &["Permission denied (publickey)", "ssh permission denied"]),
    st("st-20", "fatal: Authentication failed",                            &["Authentication failed", "git authentication failed"]),
    // Code Snippet (cs-*)
    cs("cs-01", "if let Some headers Authorization",                       &["self.headers.get(Authorization)", "Authorization header check"]),
    cs("cs-02", "tokio::spawn async move",                                 &["tokio::spawn(async move", "spawn async move"]),
    cs("cs-03", "let mut conn pool acquire await",                         &["pool.acquire().await", "conn = pool.acquire"]),
    cs("cs-04", "useState initial value",                                  &["useState(initialValue)", "useState hook init"]),
    cs("cs-05", "async fn handler Request Response",                       &["async fn handler", "fn handler Request Response"]),
    cs("cs-06", "df groupby user_id agg",                                  &["df.groupby user_id", "pandas groupby user_id"]),
    cs("cs-07", "ctx Done channel",                                        &["ctx.Done()", "select ctx.Done"]),
    cs("cs-08", "match self state Active",                                 &["State::Active", "match self.state"]),
    cs("cs-09", "axios interceptors response use",                         &["axios.interceptors.response.use", "axios interceptors"]),
    cs("cs-10", "derive Debug Serialize Deserialize",                      &["derive(Debug, Serialize, Deserialize)", "derive Debug Serialize Deserialize"]),
    cs("cs-11", "for i item vec iter enumerate",                           &["vec.iter().enumerate()", "iter enumerate loop"]),
    cs("cs-12", "redis PubSub channel",                                    &["redis.PubSub()", "redis pubsub init"]),
    cs("cs-13", "Component selector tag",                                  &["Component selector", "Angular Component selector"]),
    cs("cs-14", "DB exec SQL_INSERT_USER",                                 &["DB.exec(SQL_INSERT_USER)", "exec SQL_INSERT_USER"]),
    cs("cs-15", "if err nil return nil err go",                            &["if err != nil { return nil, err }", "go err return"]),
    cs("cs-16", "try JSON parse body catch",                               &["JSON.parse(body)", "try JSON.parse"]),
    cs("cs-17", "ChaCha20Rng seed_from_u64",                               &["ChaCha20Rng::seed_from_u64", "chacha20 seed"]),
    cs("cs-18", "flask route api v1 health",                               &["app.route(/api/v1/health)", "flask route health"]),
    cs("cs-19", "ErrorKind WouldBlock io",                                 &["io::ErrorKind::WouldBlock", "WouldBlock ErrorKind"]),
    cs("cs-20", "return new Promise resolve reject",                       &["new Promise((resolve, reject)", "Promise resolve reject"]),
    // Tool-typed (tt-*)
    tt("tt-01", "tool Edit path src parser.rs",                            &["src/parser.rs Edit", "Edit src/parser.rs"]),
    tt("tt-02", "tool Bash command cargo test",                            &["cargo test bash", "bash cargo test"]),
    tt("tt-03", "tool Read path Cargo.toml",                               &["Cargo.toml Read", "Read Cargo.toml"]),
    tt("tt-04", "tool Grep pattern TODO",                                  &["Grep TODO", "grep pattern TODO"]),
    tt("tt-05", "tool Write path README.md",                               &["README.md Write", "Write README.md"]),
    tt("tt-06", "tool Edit path src main.rs",                              &["src/main.rs Edit", "Edit src/main.rs"]),
    tt("tt-07", "tool Bash command git status",                            &["git status bash", "bash git status"]),
    tt("tt-08", "tool Read path package.json",                             &["package.json Read", "Read package.json"]),
    tt("tt-09", "tool Grep pattern fixme",                                 &["Grep fixme", "grep pattern fixme"]),
    tt("tt-10", "tool Edit path tests integration.rs",                     &["tests/integration.rs Edit", "Edit integration test"]),
    tt("tt-11", "tool Bash command npm install",                           &["npm install bash", "bash npm install"]),
    tt("tt-12", "tool Read path src lib.rs",                               &["src/lib.rs Read", "Read src/lib.rs"]),
    tt("tt-13", "tool Write path src config.rs",                           &["src/config.rs Write", "Write src/config.rs"]),
    tt("tt-14", "tool Edit path Cargo.lock",                               &["Cargo.lock Edit", "Edit Cargo.lock"]),
    tt("tt-15", "tool Bash command docker build",                          &["docker build bash", "bash docker build"]),
    tt("tt-16", "tool Grep pattern unwrap",                                &["Grep unwrap", "grep pattern unwrap"]),
    tt("tt-17", "tool MultiEdit path src repo.rs",                         &["src/repo.rs MultiEdit", "MultiEdit repo.rs"]),
    tt("tt-18", "tool Bash command make test",                             &["make test bash", "bash make test"]),
    tt("tt-19", "tool Read path docker-compose.yml",                       &["docker-compose.yml Read", "Read docker-compose"]),
    tt("tt-20", "tool Edit path src db.rs",                                &["src/db.rs Edit", "Edit src/db.rs"]),
    // Symbolic / file path (sp-*)
    sp("sp-01", "serialize_with_options",                                  &["fn serialize_with_options", "serialize_with_options self"]),
    sp("sp-02", "crates teramind-core src redact.rs",                      &["crates/teramind-core/src/redact.rs", "redact.rs"]),
    sp("sp-03", "validate_input",                                          &["fn validate_input", "validate_input("]),
    sp("sp-04", "AuthMiddleware",                                          &["class AuthMiddleware", "AuthMiddleware {"]),
    sp("sp-05", "openapi yaml",                                            &["openapi.yaml", "openapi spec"]),
    sp("sp-06", "FeatureFlag enum",                                        &["enum FeatureFlag", "FeatureFlag::"]),
    sp("sp-07", "scripts release.sh",                                      &["scripts/release.sh", "release.sh"]),
    sp("sp-08", "Datadog_client",                                          &["Datadog_client", "Datadog_client.send"]),
    sp("sp-09", "k8s deployment.yaml",                                     &["k8s/deployment.yaml", "deployment.yaml"]),
    sp("sp-10", "format_event_payload",                                    &["fn format_event_payload", "format_event_payload("]),
    sp("sp-11", "Prometheus Counter",                                      &["Prometheus.Counter", "Counter("]),
    sp("sp-12", "lib cache invalidator.go",                                &["lib/cache/invalidator.go", "invalidator.go"]),
    sp("sp-13", "RateLimitExceeded",                                       &["RateLimitExceeded", "Err::RateLimitExceeded"]),
    sp("sp-14", "src utils url.ts",                                        &["src/utils/url.ts", "utils/url.ts"]),
    sp("sp-15", "parse_iso8601",                                           &["fn parse_iso8601", "parse_iso8601("]),
    sp("sp-16", "WorkerHealth struct",                                     &["struct WorkerHealth", "WorkerHealth {"]),
    sp("sp-17", ".github workflows ci.yml",                                &[".github/workflows/ci.yml", "ci.yml"]),
    sp("sp-18", "redactPII",                                               &["redactPII", "function redactPII"]),
    sp("sp-19", "internal auth jwt.go",                                    &["internal/auth/jwt.go", "auth/jwt.go"]),
    sp("sp-20", "EventBus publish",                                        &["EventBus.publish", "EventBus.publish("]),
];

const fn nl(id: &'static str, text: &'static str, triggers: &'static [&'static str]) -> QueryBankEntry {
    QueryBankEntry { id, class: QueryClass::NaturalLanguage, text, triggers }
}
const fn st(id: &'static str, text: &'static str, triggers: &'static [&'static str]) -> QueryBankEntry {
    QueryBankEntry { id, class: QueryClass::StackTrace, text, triggers }
}
const fn cs(id: &'static str, text: &'static str, triggers: &'static [&'static str]) -> QueryBankEntry {
    QueryBankEntry { id, class: QueryClass::CodeSnippet, text, triggers }
}
const fn tt(id: &'static str, text: &'static str, triggers: &'static [&'static str]) -> QueryBankEntry {
    QueryBankEntry { id, class: QueryClass::ToolTyped, text, triggers }
}
const fn sp(id: &'static str, text: &'static str, triggers: &'static [&'static str]) -> QueryBankEntry {
    QueryBankEntry { id, class: QueryClass::SymbolicPath, text, triggers }
}
```

- [ ] **Step 4: Run the validation tests**

Run: `cargo test -p teramind-search-eval --test queries_bank`
Expected: 4 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-search-eval/src/queries_bank.rs crates/teramind-search-eval/tests/queries_bank.rs
git commit -m "feat(search-eval): query bank — 100 queries across 5 classes"
```

---

## Section 6 — Eval harness

### Task 6.1: Spin up PG and run queries

**Files:**
- Modify: `crates/teramind-search-eval/src/harness.rs`

- [ ] **Step 1: Replace the stub with the real harness**

Replace `crates/teramind-search-eval/src/harness.rs` with:

```rust
//! Eval harness: load corpus into a throwaway PG, run every query through
//! `SearchRepo::fts_turns` + `trgm_diffs`, capture ranked hit IDs.

use crate::corpus;
use crate::queries_bank::QUERIES;
use crate::reporter;
use crate::types::CorpusSize;
use std::path::Path;
use std::time::Instant;
use teramind_db::repos::SearchRepo;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};

pub async fn run(corpus_root: &Path, out_dir: &Path) -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let sup = PgSupervisor::start(tmp.path().join("pgdata"), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let cor = corpus::load(corpus_root)?;
    let size = CorpusSize {
        sessions:   cor.sessions.len()   as u32,
        turns:      cor.turns.len()      as u32,
        tool_calls: cor.tool_calls.len() as u32,
        file_diffs: cor.file_diffs.len() as u32,
    };
    corpus::ingest(&pool, &cor).await?;

    let search = SearchRepo::new(pool.clone());
    let qrels = load_qrels(corpus_root)?;

    let mut per_query: Vec<reporter::PerQuery> = Vec::with_capacity(QUERIES.len());
    let mut latencies: Vec<u128> = Vec::with_capacity(QUERIES.len());
    for q in QUERIES {
        let started = Instant::now();
        let fts = search.fts_turns(q.text, 10).await.unwrap_or_default();
        let diffs = search.trgm_diffs(q.text, 10).await.unwrap_or_default();
        let elapsed = started.elapsed().as_millis();
        latencies.push(elapsed);

        let mut hit_ids: Vec<String> = Vec::with_capacity(20);
        for f in &fts   { hit_ids.push(format!("turn:{}", f.turn_id)); }
        for d in &diffs { hit_ids.push(format!("diff:{}", d.diff_id)); }

        let relevance = relevance_for(&qrels, q.id, &hit_ids);
        let total_rel = qrels.judgments
            .get(q.id)
            .map(|v| v.iter().filter(|j| j.grade > 0).count() as u32)
            .unwrap_or(0);

        per_query.push(reporter::PerQuery {
            id: q.id.into(),
            class: q.class,
            relevance,
            total_relevant: total_rel,
        });
    }
    sup.shutdown().await?;

    latencies.sort();
    let p95_ms = percentile_u32(&latencies, 95);

    let report = reporter::aggregate(&per_query, size, p95_ms);
    reporter::write_results(out_dir, &report)?;
    println!(
        "teramind-search-eval: nDCG@10={:.3}  MRR={:.3}  p95={}ms  ({} queries)",
        report.overall.ndcg_at_10,
        report.overall.mrr,
        report.query_latency_p95_ms,
        report.overall.n_queries,
    );
    Ok(())
}

fn load_qrels(root: &Path) -> anyhow::Result<crate::types::QrelsFile> {
    let path = root.join("qrels.toml");
    if !path.exists() { return Ok(Default::default()); }
    let body = std::fs::read_to_string(&path)?;
    Ok(toml::from_str(&body)?)
}

fn relevance_for(qrels: &crate::types::QrelsFile, qid: &str, hit_ids: &[String]) -> Vec<u32> {
    let judgments = match qrels.judgments.get(qid) {
        Some(v) => v,
        None => return hit_ids.iter().map(|_| 0).collect(),
    };
    let map: std::collections::HashMap<&str, u32> = judgments.iter()
        .map(|j| (j.item.as_str(), j.grade))
        .collect();
    hit_ids.iter().map(|h| *map.get(h.as_str()).unwrap_or(&0)).collect()
}

fn percentile_u32(sorted_ms: &[u128], pct: u32) -> u32 {
    if sorted_ms.is_empty() { return 0; }
    let idx = ((sorted_ms.len() as f64) * (pct as f64) / 100.0).ceil() as usize;
    let idx = idx.min(sorted_ms.len() - 1);
    sorted_ms[idx] as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relevance_for_returns_zero_for_unjudged_query() {
        let qrels = crate::types::QrelsFile::default();
        let r = relevance_for(&qrels, "nl-99", &["turn:abc".into(), "diff:def".into()]);
        assert_eq!(r, vec![0, 0]);
    }

    #[test]
    fn percentile_handles_short_inputs() {
        assert_eq!(percentile_u32(&[1, 2, 3, 4], 95), 4);
        assert_eq!(percentile_u32(&[], 95), 0);
    }
}
```

- [ ] **Step 2: Run the unit tests**

Run: `cargo test -p teramind-search-eval harness`
Expected: 2 tests PASS.

- [ ] **Step 3: Smoke check builds**

Run: `cargo check -p teramind-search-eval`
Expected: succeeds.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-search-eval/src/harness.rs
git commit -m "feat(search-eval): eval harness driving SearchRepo against ephemeral PG"
```

---

## Section 7 — Reporter

### Task 7.1: Aggregate per-query results + emit JSON & Markdown

**Files:**
- Modify: `crates/teramind-search-eval/src/reporter.rs`

- [ ] **Step 1: Replace the stub**

Replace `crates/teramind-search-eval/src/reporter.rs` with:

```rust
//! Per-query and aggregated metric computation + scorecard emission.

use crate::metrics::{mrr, ndcg_at_k, precision_at_k, recall_at_k};
use crate::types::{CorpusSize, EvalReport, MetricsRow, QueryClass};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct PerQuery {
    pub id: String,
    pub class: QueryClass,
    pub relevance: Vec<u32>,
    pub total_relevant: u32,
}

pub fn aggregate(per_query: &[PerQuery], size: CorpusSize, p95_ms: u32) -> EvalReport {
    let overall = row_for(per_query.iter());
    let mut by_class: BTreeMap<QueryClass, MetricsRow> = BTreeMap::new();
    for class in QueryClass::all() {
        let slice = per_query.iter().filter(|p| p.class == *class);
        by_class.insert(*class, row_for(slice));
    }
    EvalReport { overall, by_class, query_latency_p95_ms: p95_ms, corpus_size: size }
}

fn row_for<'a, I: Iterator<Item = &'a PerQuery>>(iter: I) -> MetricsRow {
    let mut n = 0usize;
    let mut ndcg = 0.0; let mut mrr_sum = 0.0;
    let mut p5 = 0.0; let mut p10 = 0.0; let mut r10 = 0.0;
    for p in iter {
        n += 1;
        ndcg    += ndcg_at_k(&p.relevance, 10);
        mrr_sum += mrr(&p.relevance);
        p5      += precision_at_k(&p.relevance, 5);
        p10     += precision_at_k(&p.relevance, 10);
        r10     += recall_at_k(&p.relevance, 10, p.total_relevant);
    }
    if n == 0 {
        return MetricsRow {
            n_queries: 0, ndcg_at_10: 0.0, mrr: 0.0, p_at_5: 0.0, p_at_10: 0.0, r_at_10: 0.0,
        };
    }
    let denom = n as f64;
    MetricsRow {
        n_queries: n,
        ndcg_at_10: ndcg / denom,
        mrr:        mrr_sum / denom,
        p_at_5:     p5 / denom,
        p_at_10:    p10 / denom,
        r_at_10:    r10 / denom,
    }
}

pub fn write_results(dir: &Path, report: &EvalReport) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(dir.join("eval-results.json"), json)?;
    std::fs::write(dir.join("eval-scorecard.md"), render_markdown(report))?;
    Ok(())
}

pub fn render_markdown(report: &EvalReport) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    writeln!(s, "# Teramind Search Eval Scorecard\n").unwrap();
    writeln!(
        s,
        "Corpus: {} sessions / {} turns / {} tool calls / {} diffs",
        report.corpus_size.sessions, report.corpus_size.turns,
        report.corpus_size.tool_calls, report.corpus_size.file_diffs,
    ).unwrap();
    writeln!(s, "p95 latency per query: {} ms\n", report.query_latency_p95_ms).unwrap();

    writeln!(s, "## Overall\n").unwrap();
    writeln!(s, "| n | nDCG@10 | MRR | P@5 | P@10 | R@10 |").unwrap();
    writeln!(s, "|---|---:|---:|---:|---:|---:|").unwrap();
    write_row(&mut s, &report.overall);

    writeln!(s, "\n## Per class\n").unwrap();
    writeln!(s, "| Class | n | nDCG@10 | MRR | P@5 | P@10 | R@10 |").unwrap();
    writeln!(s, "|---|---|---:|---:|---:|---:|---:|").unwrap();
    for (class, row) in &report.by_class {
        write!(s, "| {:?} ", class).unwrap();
        writeln!(
            s, "| {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |",
            row.n_queries, row.ndcg_at_10, row.mrr, row.p_at_5, row.p_at_10, row.r_at_10,
        ).unwrap();
    }
    s
}

fn write_row(s: &mut String, row: &MetricsRow) {
    use std::fmt::Write;
    writeln!(
        s, "| {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |",
        row.n_queries, row.ndcg_at_10, row.mrr, row.p_at_5, row.p_at_10, row.r_at_10,
    ).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_overall_averages_metrics() {
        let pq = vec![
            PerQuery { id: "a".into(), class: QueryClass::NaturalLanguage, relevance: vec![2, 0, 0, 1], total_relevant: 3 },
            PerQuery { id: "b".into(), class: QueryClass::NaturalLanguage, relevance: vec![0, 1, 2, 0], total_relevant: 3 },
        ];
        let size = CorpusSize { sessions: 0, turns: 0, tool_calls: 0, file_diffs: 0 };
        let r = aggregate(&pq, size, 0);
        assert_eq!(r.overall.n_queries, 2);
        assert!(r.overall.ndcg_at_10 > 0.0 && r.overall.ndcg_at_10 < 1.0);
        // MRR: 1/1 + 1/2 averaged = 0.75
        assert!((r.overall.mrr - 0.75).abs() < 1e-9);
    }

    #[test]
    fn aggregate_zero_rows_for_classes_without_queries() {
        let pq: Vec<PerQuery> = Vec::new();
        let size = CorpusSize { sessions: 0, turns: 0, tool_calls: 0, file_diffs: 0 };
        let r = aggregate(&pq, size, 0);
        assert_eq!(r.overall.n_queries, 0);
        for class in QueryClass::all() {
            assert_eq!(r.by_class[class].n_queries, 0);
        }
    }

    #[test]
    fn markdown_contains_per_class_rows() {
        let pq = vec![PerQuery { id: "a".into(), class: QueryClass::CodeSnippet, relevance: vec![1], total_relevant: 1 }];
        let r = aggregate(&pq, CorpusSize { sessions: 1, turns: 1, tool_calls: 0, file_diffs: 0 }, 5);
        let md = render_markdown(&r);
        assert!(md.contains("Overall"));
        assert!(md.contains("Per class"));
        assert!(md.contains("CodeSnippet"));
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramind-search-eval reporter`
Expected: 3 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-search-eval/src/reporter.rs
git commit -m "feat(search-eval): aggregator + JSON/Markdown scorecard"
```

---

## Section 8 — Regression gates

### Task 8.1: `compare` + thresholds

**Files:**
- Modify: `crates/teramind-search-eval/src/gates.rs`

- [ ] **Step 1: Replace the stub**

Replace `crates/teramind-search-eval/src/gates.rs` with:

```rust
//! Regression-gate comparison.
//!
//! Thresholds (per spec §9.5):
//!   * nDCG@10 (overall): must not drop more than 2 pp vs baseline.
//!   * nDCG@10 (any class): must not drop more than 5 pp vs baseline.
//!   * MRR (overall): must not drop more than 0.03 absolute.
//!   * eval p95 latency: must not exceed 3000 ms per query.

use crate::types::{Baseline, EvalReport};
use std::path::Path;

pub const NDCG_OVERALL_DROP: f64 = 0.02;
pub const NDCG_PER_CLASS_DROP: f64 = 0.05;
pub const MRR_OVERALL_DROP: f64 = 0.03;
pub const P95_LATENCY_CEILING_MS: u32 = 3_000;

#[derive(Debug, PartialEq)]
pub struct GateOutcome {
    pub passed: bool,
    pub failures: Vec<String>,
}

pub fn check(report: &EvalReport, baseline: &Baseline) -> GateOutcome {
    let mut failures = Vec::new();

    let ndcg_drop = baseline.overall.ndcg_at_10 - report.overall.ndcg_at_10;
    if ndcg_drop > NDCG_OVERALL_DROP + 1e-9 {
        failures.push(format!(
            "overall nDCG@10 dropped {:.4} (limit {:.4})",
            ndcg_drop, NDCG_OVERALL_DROP,
        ));
    }
    let mrr_drop = baseline.overall.mrr - report.overall.mrr;
    if mrr_drop > MRR_OVERALL_DROP + 1e-9 {
        failures.push(format!(
            "overall MRR dropped {:.4} (limit {:.4})",
            mrr_drop, MRR_OVERALL_DROP,
        ));
    }
    for (class, bl) in &baseline.by_class {
        if let Some(rep) = report.by_class.get(class) {
            let drop = bl.ndcg_at_10 - rep.ndcg_at_10;
            if drop > NDCG_PER_CLASS_DROP + 1e-9 {
                failures.push(format!(
                    "class {:?} nDCG@10 dropped {:.4} (limit {:.4})",
                    class, drop, NDCG_PER_CLASS_DROP,
                ));
            }
        }
    }
    if report.query_latency_p95_ms > P95_LATENCY_CEILING_MS {
        failures.push(format!(
            "p95 latency {} ms exceeds ceiling {} ms",
            report.query_latency_p95_ms, P95_LATENCY_CEILING_MS,
        ));
    }
    GateOutcome { passed: failures.is_empty(), failures }
}

pub fn compare(results_path: &Path, baseline_path: &Path, update: bool) -> anyhow::Result<()> {
    let results: EvalReport = serde_json::from_slice(&std::fs::read(results_path)?)?;

    if update {
        std::fs::write(baseline_path, serde_json::to_string_pretty(&results)?)?;
        println!(
            "teramind-search-eval: baseline updated -> {}",
            baseline_path.display(),
        );
        return Ok(());
    }

    if !baseline_path.exists() {
        println!(
            "teramind-search-eval: no baseline; pass --update-baseline to seed one.",
        );
        return Ok(());
    }
    let baseline: Baseline = serde_json::from_slice(&std::fs::read(baseline_path)?)?;
    let outcome = check(&results, &baseline);
    if outcome.passed {
        println!("teramind-search-eval: all gates passed");
        Ok(())
    } else {
        for f in &outcome.failures {
            eprintln!("teramind-search-eval: gate failure: {f}");
        }
        anyhow::bail!("regression gate tripped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CorpusSize, MetricsRow, QueryClass};
    use std::collections::BTreeMap;

    fn report(ndcg: f64, mrr: f64, class_ndcg: f64) -> EvalReport {
        let mut by_class = BTreeMap::new();
        by_class.insert(QueryClass::NaturalLanguage, MetricsRow {
            n_queries: 20, ndcg_at_10: class_ndcg, mrr: 0.5, p_at_5: 0.5, p_at_10: 0.4, r_at_10: 0.3,
        });
        EvalReport {
            overall: MetricsRow { n_queries: 100, ndcg_at_10: ndcg, mrr, p_at_5: 0.5, p_at_10: 0.4, r_at_10: 0.3 },
            by_class,
            query_latency_p95_ms: 100,
            corpus_size: CorpusSize { sessions: 500, turns: 2500, tool_calls: 5000, file_diffs: 500 },
        }
    }

    #[test]
    fn equal_metrics_pass() {
        let b = report(0.80, 0.70, 0.85);
        let r = report(0.80, 0.70, 0.85);
        assert!(check(&r, &b).passed);
    }

    #[test]
    fn overall_ndcg_drop_two_pp_passes_marginally() {
        let b = report(0.80, 0.70, 0.85);
        let r = report(0.78, 0.70, 0.85); // exactly 2 pp drop
        let o = check(&r, &b);
        assert!(o.passed, "{:?}", o.failures);
    }

    #[test]
    fn overall_ndcg_drop_more_than_two_pp_fails() {
        let b = report(0.80, 0.70, 0.85);
        let r = report(0.77, 0.70, 0.85);
        let o = check(&r, &b);
        assert!(!o.passed);
        assert!(o.failures[0].contains("nDCG@10"));
    }

    #[test]
    fn class_ndcg_drop_more_than_five_pp_fails() {
        let b = report(0.80, 0.70, 0.85);
        let r = report(0.80, 0.70, 0.79);
        let o = check(&r, &b);
        assert!(!o.passed);
        assert!(o.failures[0].contains("class"));
    }

    #[test]
    fn mrr_drop_more_than_003_fails() {
        let b = report(0.80, 0.70, 0.85);
        let r = report(0.80, 0.66, 0.85);
        let o = check(&r, &b);
        assert!(!o.passed);
        assert!(o.failures[0].contains("MRR"));
    }

    #[test]
    fn p95_latency_above_ceiling_fails() {
        let b = report(0.80, 0.70, 0.85);
        let mut r = report(0.80, 0.70, 0.85);
        r.query_latency_p95_ms = 3_500;
        let o = check(&r, &b);
        assert!(!o.passed);
        assert!(o.failures[0].contains("p95"));
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramind-search-eval gates`
Expected: 6 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-search-eval/src/gates.rs
git commit -m "feat(search-eval): regression gates per spec thresholds"
```

---

## Section 9 — Corpus generation + baseline seed

### Task 9.1: Generate corpus, run eval, seed baseline

**Files (committed outputs):**
- Create: `benches/search-eval/corpus/{sessions,turns,tool_calls,file_diffs}.jsonl`
- Create: `benches/search-eval/qrels.toml`
- Create: `benches/search-eval/queries.toml`
- Create: `benches/search-eval/baseline.json`
- Create: `benches/search-eval/README.md`
- Modify: `.gitignore`

- [ ] **Step 1: Generate the corpus + qrels + queries**

Run: `cargo run --release -p teramind-search-eval -- generate-corpus --scale=500`
Expected: prints `wrote 500 sessions / N turns / M diffs to benches/search-eval`. New files appear under `benches/search-eval/corpus/` plus `qrels.toml` and `queries.toml` at the root of that directory.

- [ ] **Step 2: Run the benchmark and seed `eval-results.json`**

Run: `cargo run --release -p teramind-search-eval -- run`
Expected: writes `benches/search-eval/eval-results.json` and `eval-scorecard.md`. Should finish in under 2 minutes (spec target).

- [ ] **Step 3: Seed the baseline**

Run: `cargo run --release -p teramind-search-eval -- compare-baseline --update-baseline`
Expected: writes `benches/search-eval/baseline.json` from the just-produced results.

- [ ] **Step 4: Sanity-check the gate against the seeded baseline**

Run: `cargo run --release -p teramind-search-eval -- compare-baseline`
Expected: `teramind-search-eval: all gates passed`. (The current run equals the baseline byte-for-byte.)

- [ ] **Step 5: Author the README**

Create `benches/search-eval/README.md`:

```markdown
# Teramind Search Eval Corpus

This directory holds the L5 search-effectiveness benchmark assets:

- `corpus/sessions.jsonl`, `turns.jsonl`, `tool_calls.jsonl`, `file_diffs.jsonl`
  — 500-session synthetic corpus, regenerable via the generator.
- `queries.toml` — 100 hand-curated queries across 5 intent classes
  (≥20 per class).
- `qrels.toml` — per-query relevance judgments (graded 0/1/2).
- `baseline.json` — committed metrics from `main`. PRs that touch
  search-related paths must keep metrics within spec thresholds:
  - nDCG@10 (overall): ≤ 2 pp drop
  - nDCG@10 (any class): ≤ 5 pp drop
  - MRR (overall): ≤ 0.03 absolute drop
  - p95 query latency: ≤ 3 s
- `eval-results.json` + `eval-scorecard.md` — outputs of the most recent
  local run; gitignored.

## Regenerating

```sh
cargo run --release -p teramind-search-eval -- generate-corpus --scale=500
cargo run --release -p teramind-search-eval -- run
```

## Rebaselining (intentional metric move)

If a PR genuinely improves ranking and the gates trip because the baseline
is stale, attach `[eval-baseline-update]` to the PR description AND commit
the new `baseline.json`:

```sh
cargo run --release -p teramind-search-eval -- run
cargo run --release -p teramind-search-eval -- compare-baseline --update-baseline
git add benches/search-eval/baseline.json
git commit -m "eval: rebaseline (improves nDCG@10 by X.Y pp)"
```

Reviewers can inspect the new numbers in the diff.
```

- [ ] **Step 6: Gitignore the transient outputs**

Append to `.gitignore` (create if missing):

```
benches/search-eval/eval-results.json
benches/search-eval/eval-scorecard.md
```

- [ ] **Step 7: Commit corpus + baseline + README**

```bash
git add benches/search-eval/ .gitignore
git commit -m "feat(search-eval): seed corpus + qrels + queries + baseline"
```

(The corpus JSONL files weigh ~2 MB at scale=500; manageable in git.)

---

## Section 10 — CI gate workflow

### Task 10.1: `.github/workflows/search-eval.yml`

**Files:**
- Create: `.github/workflows/search-eval.yml`

- [ ] **Step 1: Author the workflow**

Create `.github/workflows/search-eval.yml`:

```yaml
name: search-eval
on:
  pull_request:
    paths:
      - "crates/teramind-db/src/repos/search.rs"
      - "crates/teramindd/src/services/search.rs"
      - "crates/teramind-search-eval/**"
      - "benches/search-eval/**"
  schedule:
    - cron: "0 6 * * 1"   # weekly Monday 06:00 UTC
  workflow_dispatch: {}

jobs:
  eval:
    name: run L5 benchmark
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: run benchmark
        env:
          TERAMIND_LOG: warn
        run: cargo run --release -p teramind-search-eval -- run
      - name: detect rebaseline marker
        id: rebaseline
        shell: bash
        run: |
          if [ "${{ github.event_name }}" = "pull_request" ] && \
             echo "${{ github.event.pull_request.body }}" | grep -q "\[eval-baseline-update\]"; then
            echo "permitted=true" >> "$GITHUB_OUTPUT"
          else
            echo "permitted=false" >> "$GITHUB_OUTPUT"
          fi
      - name: compare against baseline
        if: steps.rebaseline.outputs.permitted != 'true'
        run: cargo run --release -p teramind-search-eval -- compare-baseline
      - name: report (rebaseline mode)
        if: steps.rebaseline.outputs.permitted == 'true'
        run: |
          echo "PR opted into [eval-baseline-update]; gates SKIPPED."
          echo "Reviewer must inspect the new baseline.json in the diff."
      - uses: actions/upload-artifact@v4
        with:
          name: eval-scorecard
          path: |
            benches/search-eval/eval-results.json
            benches/search-eval/eval-scorecard.md
          if-no-files-found: error
```

- [ ] **Step 2: Validate YAML**

Run: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/search-eval.yml'))" && echo OK`
Expected: `OK`.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/search-eval.yml
git commit -m "ci(search-eval): gate PRs touching search code + weekly run"
```

---

## Section 11 — `teramind doctor` integration

### Task 11.1: Doctor reports local-corpus nDCG@10

**Files:**
- Modify: `crates/teramind/src/commands/doctor.rs`
- Modify: `crates/teramind/Cargo.toml`

- [ ] **Step 1: Add the dependency**

In `crates/teramind/Cargo.toml`, append to `[dependencies]`:

```toml
teramind-search-eval = { path = "../teramind-search-eval" }
```

- [ ] **Step 2: Append baseline check to `doctor::run`**

Edit `crates/teramind/src/commands/doctor.rs`. At the END of `run` (before the final `Ok(())`), insert:

```rust
    if let Some(metrics) = load_local_baseline() {
        println!(
            "search baseline (last committed): nDCG@10={:.3}  MRR={:.3}  p95={}ms  ({} queries)",
            metrics.overall.ndcg_at_10,
            metrics.overall.mrr,
            metrics.query_latency_p95_ms,
            metrics.overall.n_queries,
        );
    } else {
        println!("search baseline: not seeded (run `cargo run -p teramind-search-eval -- run` then `compare-baseline --update-baseline`)");
    }
```

Then add the helper function below `run` in the same file:

```rust
fn load_local_baseline() -> Option<teramind_search_eval::types::Baseline> {
    let candidates = [
        std::path::PathBuf::from("benches/search-eval/baseline.json"),
        std::env::current_exe().ok()?.parent()?.join("../../benches/search-eval/baseline.json"),
    ];
    for path in &candidates {
        if let Ok(body) = std::fs::read(path) {
            if let Ok(b) = serde_json::from_slice(&body) {
                return Some(b);
            }
        }
    }
    None
}
```

(If `serde_json` isn't already imported in `doctor.rs`, no `use` is needed — we go through the fully qualified path. But it IS already a dependency of `teramind-cli`.)

- [ ] **Step 3: `cargo check -p teramind-cli`**

Expected: succeeds.

- [ ] **Step 4: Write a smoke test**

Append to `crates/teramind/src/commands/doctor.rs` (inside the existing test module if one exists, otherwise add one):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_local_baseline_returns_none_when_path_missing() {
        let _ = std::env::set_current_dir(std::env::temp_dir());
        assert!(load_local_baseline().is_none());
    }
}
```

- [ ] **Step 5: Run**

Run: `cargo test -p teramind-cli doctor`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/teramind/src/commands/doctor.rs crates/teramind/Cargo.toml
git commit -m "feat(cli): doctor surfaces local search-eval baseline"
```

---

## Section 12 — Final integration check

### Task 12.1: Workspace check, eval re-run, clippy

- [ ] **Step 1: Workspace check**

```bash
cargo check --workspace
cargo test --workspace --lib
cargo test -p teramind-search-eval
cargo clippy --workspace -- -D warnings
```

Expected: all pass. Fix minor lint issues inline (unused imports, `clippy::needless_clone`, etc.).

- [ ] **Step 2: Re-run the eval (sanity)**

```bash
cargo run --release -p teramind-search-eval -- run
cargo run --release -p teramind-search-eval -- compare-baseline
```

Expected: `teramind-search-eval: all gates passed` (because we just seeded the baseline from this run in §9).

- [ ] **Step 3: Validate workflow YAML**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/search-eval.yml'))"
```

Expected: no exception.

- [ ] **Step 4: Optional cleanup commit**

```bash
git add -A
git commit -m "chore: clippy cleanups for search-eval plan" || true
```

- [ ] **Step 5: STOP — do not push or open a PR.**

Defer to user approval, per Plans A–E convention.

---

## Spec coverage self-check

| Spec section / requirement | Plan task |
|---|---|
| §9.5 corpus location `benches/search-eval/` | §9 |
| §9.5 corpus contents (sessions/turns/tool_calls/file_diffs.jsonl) | §3 + §4 generator |
| §9.5 100 labelled queries in `queries.toml` | §5 |
| §9.5 ≥20 queries per class × 5 classes | §5 (validated by test) |
| §9.5 `qrels.toml` graded 0/1/2 | §4 generator emits grades 0 and 2 |
| §9.5 `baseline.json` (current `main` metrics) | §9 |
| §9.5 README authoring guide | §9 |
| §9.5 metrics: nDCG@10, MRR, P@5, P@10, R@10 | §1 + §7 |
| §9.5 `teramind-search-eval` binary writes `eval-results.json` + Markdown | §0 main + §6 + §7 |
| §9.5 runtime budget < 2 min | §9 step 2 verifies |
| §9.5 regression gate: nDCG@10 overall ≤ 2 pp drop | §8 |
| §9.5 regression gate: nDCG@10 per class ≤ 5 pp drop | §8 |
| §9.5 regression gate: MRR ≤ 0.03 absolute drop | §8 |
| §9.5 regression gate: eval p95 latency ≤ 3 s | §8 |
| §9.5 CI fires on PRs touching search paths | §10 |
| §9.5 weekly CI run otherwise | §10 |
| §9.5 `[eval-baseline-update]` PR tag opts out of gate + adopts new baseline | §10 |
| §9.5 `teramind doctor` reports local-corpus nDCG@10 | §11 |
| §9.5 corpus growth (post-v1) | out of scope, noted |
