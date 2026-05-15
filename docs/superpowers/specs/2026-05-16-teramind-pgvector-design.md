# Teramind pgvector — Design Spec

- **Status:** Approved (brainstorming complete; pending implementation plan)
- **Author:** Vahe Momjyan
- **Date:** 2026-05-16
- **Scope:** Spec #7 of the Teramind product roadmap. Adds semantic search to the existing local-first substrate.

---

## 1. Background and motivation

Teramind Core (specs A–F) ships full-fidelity trace capture, lexical search (`tsvector` full-text + `pg_trgm`), an MCP-driven recall surface, and an L5 effectiveness benchmark with regression gates. Lexical retrieval works well for queries that share vocabulary with the corpus, but degrades when a user asks "how did we make auth resilient to clock skew" and the relevant turn says "rotate the JWT before expiry."

This spec adds a **semantic retrieval layer** that runs alongside the lexical paths without changing their behavior or replacing them. The architectural promise of Teramind Core — local-first, no outbound calls except by user-installed Claude — is preserved: the default embedding provider is **Ollama on `localhost`**, with a bundled in-process fallback (`fastembed-rs`) and opt-in cloud providers gated by an explicit `network_egress = true` config flag.

The L5 benchmark from Plan F serves as the empirical arbiter: a separate `--semantic` eval mode produces a parallel scorecard (`baseline-semantic.json`) so quality changes are measurable.

## 2. Goals and non-goals

### 2.1 In scope (v1.0)

- A new `embeddings` table holding per-item vectors keyed by `(item_kind, item_id, model)`. No changes to existing tables.
- `pgvector` extension enabled in the embedded Postgres, HNSW index with cosine distance pre-created.
- An `EmbeddingProvider` trait with three implementations: **`OllamaProvider`** (default), **`FastEmbedProvider`** (in-process fallback), and a stub `CloudProvider` shape that gates Anthropic/OpenAI/Voyage behind `network_egress = true` (full cloud-provider wiring is v1.1).
- A new daemon service `embedding_worker` that polls a `traces_to_embed` view, batches rows, applies the existing `Redactor`, calls the active provider, and persists vectors. Async; never blocks ingest or search.
- `SearchRepo` gains `vector_search_turns` and `vector_search_diffs` returning rows ranked by cosine similarity.
- The `services/search.rs::final_score` blend gains a `semantic` term with default weight `0.0` (i.e. semantic is **off by default**). Users opt in via `~/.config/teramind/search.toml`.
- `teramind-search-eval` gains a `--semantic` flag that runs the harness with `semantic_weight > 0`, drives the worker against the throwaway PG, and writes a parallel scorecard.
- A new fail-soft CI job `eval-semantic` that gates the semantic path against `baseline-semantic.json`.
- `teramind doctor` surfaces embedding provider health + backlog.

### 2.2 Explicit non-goals (deferred to follow-on revisions of this spec)

- Cloud provider implementations beyond the trait surface and config gating (v1.1).
- Real-user paraphrase corpus contribution to L5 (v1.0.1).
- Chunking of long inputs that exceed `provider.max_tokens()` (v1.1; v1.0 truncates).
- Per-project model overrides (v1.1 if asked).
- Hybrid retrieval re-ranking via cross-encoder (v2).
- Fine-tuning the embedding model.

### 2.3 Success criteria

1. After `teramind init` on a host with Ollama serving `nomic-embed-text`, the embedding worker fills `embeddings` rows for new turns and file_diffs within 10 s of ingest, with zero impact on ingest latency.
2. With `semantic_weight = 0.4` in `search.toml`, semantic queries return relevant turns whose lexical content does NOT contain the query terms (paraphrase recall).
3. `teramind-search-eval run --semantic` completes in under 3 minutes on a 500-session corpus with Ollama running locally.
4. The default (lexical-only) L5 gate from Plan F remains unaffected. Semantic regression is observed independently via `baseline-semantic.json`.
5. If Ollama is offline, the daemon stays up, the worker quietly backs off, and `teramind doctor` surfaces the outage. Search degrades to lexical-only on a per-query basis with a warning in logs.

## 3. High-level architecture

Two new components added to the existing daemon layout. No new processes; no IPC contract changes.

```
╔════════════════════════════════════════════════════════════════════╗
║                       teramindd (unchanged outer shape)             ║
║                                                                    ║
║   ingest     fs_watcher    storage_stats    search                  ║
║      │           │              │              │                    ║
║      ▼           ▼              ▼              ▼                    ║
║   ┌─────────────────────────────────────────────────────────┐      ║
║   │ Postgres pool                                            │     ║
║   │                                                          │     ║
║   │  ┌─────────────┐  ┌─────────────┐                       │     ║
║   │  │ sessions    │  │ embeddings  │ ← NEW                 │     ║
║   │  │ turns       │  │  (item_kind,│                       │     ║
║   │  │ tool_calls  │  │   item_id,  │                       │     ║
║   │  │ file_diffs  │  │   model,    │                       │     ║
║   │  │ skills      │  │   vec(768)) │                       │     ║
║   │  └─────────────┘  └─────────────┘                       │     ║
║   │   traces_fts (MV)   traces_to_embed (VIEW) ← NEW        │     ║
║   └─────────────────────────────────────────────────────────┘     ║
║                                ▲                                    ║
║                                │ polls + bulk-inserts               ║
║   ┌────────────────────────┐   │                                    ║
║   │  embedding_worker      │───┘   ← NEW                            ║
║   │  (poll → redact →      │                                        ║
║   │   provider.embed →     │                                        ║
║   │   bulk insert)         │                                        ║
║   └───────────┬────────────┘                                        ║
║               │                                                     ║
║               ▼                                                     ║
║   ┌────────────────────────┐                                        ║
║   │  EmbeddingProvider     │  ← NEW trait, three impls              ║
║   │   OllamaProvider       │     (default)                          ║
║   │   FastEmbedProvider    │     (in-process fallback)              ║
║   │   CloudProvider stub   │     (Anthropic/OpenAI/Voyage in v1.1)  ║
║   └────────────────────────┘                                        ║
╚════════════════════════════════════════════════════════════════════╝

                                    │
                                    ▼
                       ┌─────────────────────────┐
                       │ http://localhost:11434  │  (Ollama)
                       └─────────────────────────┘
```

**Layer responsibilities (delta over Core):**

- **`embeddings` table** — separate from `turns`/`file_diffs`. Sparse-fill, model-versioned, survives provider swaps. No FK to parent tables (orphans pruned by a daily sweeper).
- **`embedding_worker`** — single-writer pipeline mirroring the `ingest` service's discipline. Health-checks the provider; pauses on outage; resumes without state machine.
- **`EmbeddingProvider` trait** — concrete impls live under `crates/teramindd/src/services/embed/`. The trait + shared types live in `crates/teramind-core/src/embed.rs` so the eval crate can mock providers without depending on `teramindd`.
- **`SearchRepo` extensions** — additive methods that issue pgvector queries. Existing methods (`fts_turns`, `trgm_diffs`, `trgm_skills`, `recent_turns_in_project`, `diff_excerpts_for_cwd_files`) are unchanged.
- **`services/search.rs` blend** — gains a `semantic` term; when `weights.semantic > 0` the search path embeds the query and runs vector search in parallel with existing lexical queries via `tokio::try_join!`. On embedding-query failure, semantic is treated as zero for that call.

## 4. Components and storage

### 4.1 Workspace layout (delta)

```
crates/teramind-core/
└── src/
    └── embed.rs                ← NEW: EmbeddingProvider trait + shared types

crates/teramindd/
└── src/services/
    ├── embed/
    │   ├── mod.rs              ← NEW: provider registry + factory
    │   ├── ollama.rs           ← NEW: OllamaProvider impl
    │   ├── fastembed.rs        ← NEW: FastEmbedProvider impl
    │   └── cloud.rs            ← NEW (v1.1 wiring; v1.0 stub)
    ├── embedding_worker.rs     ← NEW: poll/redact/embed/persist loop
    └── search.rs               ← MODIFIED: semantic blend term

crates/teramind-db/
├── migrations/
│   └── 20260516000001_embeddings.sql  ← NEW
└── src/repos/
    └── search.rs               ← MODIFIED: vector_search_{turns,diffs}

crates/teramind-search-eval/
└── src/
    ├── harness.rs              ← MODIFIED: --semantic flag wiring
    └── gates.rs                ← MODIFIED: parallel baseline path
```

### 4.2 Storage: schema

```sql
-- Migration: 20260516000001_embeddings.sql

CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE embeddings (
  id           bigserial PRIMARY KEY,
  item_kind    text NOT NULL CHECK (item_kind IN ('turn', 'file_diff')),
  item_id      uuid NOT NULL,
  model        text NOT NULL,                    -- e.g. "ollama:nomic-embed-text"
  dim          integer NOT NULL,                 -- 768 for nomic-embed-text
  embedding    vector(768) NOT NULL,
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (item_kind, item_id, model)
);

CREATE INDEX embeddings_lookup ON embeddings (item_kind, item_id);
CREATE INDEX embeddings_model  ON embeddings (model);

-- HNSW vector index, cosine. Cheap at <10k rows, ready for growth.
CREATE INDEX embeddings_hnsw ON embeddings
  USING hnsw (embedding vector_cosine_ops)
  WITH (m = 16, ef_construction = 64);

-- View: rows that lack an embedding for *some* model. The worker
-- composes the model filter at query time so v1.0 keeps the SQL trivial.
CREATE VIEW traces_to_embed AS
SELECT 'turn'      AS kind,
       t.id        AS item_id,
       COALESCE(t.user_prompt, '') || ' ' || COALESCE(t.assistant_text, '') AS text
FROM   turns t
UNION ALL
SELECT 'file_diff' AS kind,
       d.id        AS item_id,
       d.pre_excerpt || ' ' || d.post_excerpt AS text
FROM   file_diffs d;
```

**Key decisions:**

- **Separate `embeddings` table, not a column on the parent tables.** Decouples vector storage from main data model; lets us hold multiple embeddings per item during provider transitions; sparse-fill friendly.
- **Fixed `vector(768)`.** Matches `nomic-embed-text`. Migrating to a different-dim model requires an explicit schema bump.
- **HNSW pre-created.** Cosine distance. ~10% insert overhead at <10k rows, transparent at small scale.
- **No FK on `item_id`.** Embeddings survive cascading deletes; a daily sweeper drops orphans (`WHERE NOT EXISTS ... turns/file_diffs`). Acceptable lag: ≤24h.

### 4.3 `EmbeddingProvider` trait

```rust
// crates/teramind-core/src/embed.rs

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn model_id(&self) -> &str;            // e.g. "ollama:nomic-embed-text"
    fn dimension(&self) -> usize;          // e.g. 768
    fn max_tokens(&self) -> usize;         // truncation threshold
    fn distance_metric(&self) -> DistanceMetric;  // Cosine | Dot

    /// Cheap probe; returns Ok on a working provider.
    async fn health_check(&self) -> Result<()>;

    /// Embed a batch. The provider's batch_size cap is honored by the caller.
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}

pub enum DistanceMetric { Cosine, Dot }
```

**Implementations:**

| Provider | Wire | Setup |
|---|---|---|
| `OllamaProvider` (**default**) | HTTP POST `http://localhost:11434/api/embeddings` per text | User runs `ollama pull nomic-embed-text` once. Health check is `GET /api/version`. |
| `FastEmbedProvider` | In-process via `fastembed-rs` crate | Bundled. Downloads model weights to `~/.local/share/teramind/embed-models/` on first use (~150 MB). No external runtime. |
| `CloudProvider` | HTTPS to vendor API | v1.0 ships the trait shape + config validation; full HTTPS plumbing is v1.1. |

**Provider factory** in `crates/teramindd/src/services/embed/mod.rs` reads `~/.config/teramind/embed.toml` and constructs the active provider. Switching providers triggers a re-embed loop because `embeddings.model` is part of the unique key — old rows stay for rollback; the worker just fills the missing-for-this-model slots.

### 4.4 `embedding_worker` service

```
poll_interval = 5s
batch_size    = 32                 (configurable, capped by provider.max_batch())
max_throughput_per_min = 1000      (configurable, default; protects against thundering herd)

loop {
    sleep(poll_interval)
    if !provider.health_check().await.is_ok() {
        stats.embedding_errors += 1; continue
    }
    rows = pool.fetch_to_embed(active_model, batch_size).await
    if rows.is_empty() { continue }

    // Redaction-before-embedding (same rule as ingest persistence).
    texts = rows.iter().map(|r| redactor.apply(&r.text)).collect::<Vec<_>>()

    // Truncate to provider.max_tokens() (token-counted via a cheap heuristic
    // or the provider's tokenizer when available).
    truncated = texts.map(|t| truncate_to_tokens(t, provider.max_tokens()))

    match provider.embed(&truncated).await {
        Ok(vectors) => pool.bulk_insert_embeddings(&rows, active_model, &vectors).await?,
        Err(e) if e.is_size_exceeded() => bisect_and_retry(&rows).await,
        Err(e) => { stats.embedding_errors += 1; warn!(?e) }
    }
}
```

**Properties:**

- **Single writer** to `embeddings`. Bulk-insert uses `ON CONFLICT (item_kind, item_id, model) DO NOTHING` so concurrent ingest/search don't conflict.
- **Capture-safe.** Search and ingest never block on the worker. The worker's slowness only delays semantic-ranking accuracy on the most recent rows; lexical paths still return correct results immediately.
- **Outage-resilient.** Provider failures increment `embedding_errors`; the worker quietly resumes when health recovers.
- **Bisection-on-size.** Some providers return 413 / token-limit errors mid-batch. The worker halves the batch up to depth 4, then drops with a structured error per row.

### 4.5 Search service modifications

`crates/teramindd/src/services/search.rs::BlendWeights`:

```rust
pub struct BlendWeights {
    pub fts: f32,
    pub trgm: f32,
    pub semantic: f32,   // NEW; default 0.0
    pub recency: f32,
    pub project: f32,
}
```

`final_score` gains the term:

```rust
weights.fts      * fts_score
+ weights.trgm     * trgm_score
+ weights.semantic * semantic_score
+ weights.recency  * recency_decay
+ weights.project  * project_boost
```

`do_search` flow when `weights.semantic > 0`:

```rust
let query_emb_future = async {
    if weights.semantic > 0.0 {
        provider.embed(&[query.to_string()]).await
                .map(|v| v.into_iter().next())
                .ok().flatten()
    } else { None }
};
let (fts, trgm, query_emb) = tokio::try_join!(
    search_repo.fts_turns(query, limit),
    search_repo.trgm_diffs(query, limit),
    query_emb_future,
)?;
let semantic_hits = match query_emb {
    Some(v) => search_repo.vector_search_turns(&v, model, limit).await?,
    None    => vec![],
};
// rank_and_hydrate merges all three sources with the blend weights.
```

If the query embedding fails (Ollama down, etc.), `semantic_hits` is empty and the response's `degraded: bool` flag is set true. Capture/search never errors on the embedding path.

`SearchRepo::vector_search_turns` SQL:

```sql
SELECT t.id AS turn_id, t.session_id, t.ordinal, t.started_at,
       s.project_id,
       1.0 - (e.embedding <=> $1::vector) AS semantic_score,
       t.user_prompt, t.assistant_text
FROM   embeddings e
JOIN   turns t      ON e.item_kind = 'turn'  AND e.item_id = t.id
JOIN   sessions s   ON s.id        = t.session_id
WHERE  e.model = $2
ORDER  BY e.embedding <=> $1::vector
LIMIT  $3
```

`vector_search_diffs` mirrors the shape against `file_diffs`. Both return rows ordered by ascending cosine distance; `semantic_score = 1 - distance` is in `[0, 1]` for cosine.

## 5. Configuration

Two TOML files under `~/.config/teramind/`:

**`embed.toml`**

```toml
provider = "ollama"                  # ollama | fastembed | anthropic | openai | voyage
model    = "nomic-embed-text"        # provider-scoped model id

poll_interval_secs       = 5
batch_size               = 32
max_throughput_per_min   = 1000
orphan_sweep_interval_hr = 24

# Required to use a cloud provider. Daemon refuses to start otherwise.
network_egress = false

# Cloud-only:
# api_key_file = "~/.config/teramind/secrets.toml"
# secret_key   = "anthropic_api_key"
# max_embeddings_per_day = 10000

[ollama]
url = "http://localhost:11434"
request_timeout_ms = 10000

[fastembed]
cache_dir = "~/.local/share/teramind/embed-models"
```

**`search.toml`**

```toml
[blend]
fts       = 0.6
trgm      = 0.4
semantic  = 0.0       # OFF by default — opt-in
recency   = 0.2
project   = 0.3
```

**Validation rules:**

- `provider in {anthropic, openai, voyage}` and `network_egress = false` → daemon refuses to start with actionable error: *"egress provider configured without network_egress=true. Either flip the flag or switch to ollama/fastembed."*
- `weights.semantic > 0.0` and no `embeddings` rows exist yet → first call logs a warning, falls back to lexical, continues. The provider trait must still be configured (the query needs to be embedded), but missing index rows don't error.
- Switching `provider` or `model` does NOT migrate existing rows. Old rows remain for rollback; the worker fills the gap for the new `(item, model)` keys.

## 6. Eval extension

The L5 benchmark from Plan F gets a parallel semantic mode without disturbing the existing lexical gates.

**CLI:**

```
cargo run --release -p teramind-search-eval -- run                    # lexical, unchanged
cargo run --release -p teramind-search-eval -- run --semantic         # semantic enabled
cargo run --release -p teramind-search-eval -- run --semantic --semantic-weight 0.4
```

When `--semantic` is set:
1. Harness reads `embed.toml` to pick the provider.
2. Probes the provider; on failure, exits 2 (yellow / skipped — not a regression).
3. Loads the corpus into the throwaway PG (existing harness).
4. Spawns the `embedding_worker` against that PG, waits for `embedding_backlog == 0` (timeout: 3 min).
5. Runs all 100 queries with `semantic_weight = 0.4`.
6. Writes `eval-results-semantic.json` + `eval-scorecard-semantic.md`.

**Baselines:**

- `benches/search-eval/baseline.json` — lexical-only (unchanged from Plan F).
- `benches/search-eval/baseline-semantic.json` — committed alongside this spec's first merge to `main`, seeded by running `--semantic` once.

Both baselines are gated by the same thresholds (≤2 pp overall nDCG drop, ≤5 pp per class, MRR ≤0.03, p95 ≤3 s).

**CI workflow** — `.github/workflows/search-eval.yml` gains a sibling job:

```yaml
  eval-semantic:
    name: run L5 benchmark (semantic)
    needs: eval
    runs-on: ubuntu-22.04
    continue-on-error: true   # fail-soft until corpus expansion lands in v1.0.1
    steps:
      - uses: actions/checkout@v4
      - name: install ollama
        run: curl -fsSL https://ollama.ai/install.sh | sh
      - name: serve ollama
        run: |
          ollama serve &
          sleep 5
          ollama pull nomic-embed-text
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: run benchmark
        run: cargo run --release -p teramind-search-eval -- run --semantic
      - name: compare against baseline
        run: |
          cargo run --release -p teramind-search-eval -- compare-baseline \
            --results  benches/search-eval/eval-results-semantic.json \
            --baseline benches/search-eval/baseline-semantic.json
```

**Why fail-soft initially.** The Plan F corpus uses exact-token triggers (FTS-friendly), so semantic may not lift — or could even regress — until the corpus grows paraphrase queries. The semantic eval is shipped now as instrumentation; tightening to fail-hard is a v1.0.1 task once the corpus is expanded.

**`teramind doctor`** picks up two new lines when embeddings exist:

```
embedding provider: ollama (model=nomic-embed-text, healthy)
embedding backlog:  0 rows (last filled 14s ago)
```

If the provider is offline:

```
embedding provider: ollama (unreachable since 2026-05-16T14:22:01Z)
embedding backlog:  187 rows (worker paused)
```

## 7. Testing strategy

Five-layer model from Core spec §9.

### 7.1 L1 — Unit (pure logic, no I/O)

- Cosine-distance round-trip math (`1.0 - <=>` invariants on hand-computed vectors).
- Config parsing: `embed.toml` malformed input → actionable error; provider + `network_egress` validation matrix.
- `BlendWeights::default()` includes `semantic = 0.0`.
- `MockEmbeddingProvider` (deterministic vectors keyed by input hash) — used by Sections 4 and 6 to keep tests offline.
- Token truncation: cap respected, no panic on multi-byte input.

### 7.2 L2 — Component (per-crate, real embedded Postgres)

- Migration applies; `vector` extension loads; `embeddings_hnsw` index exists; `traces_to_embed` view returns rows.
- `SearchRepo::vector_search_turns` returns rows ordered by ascending cosine distance; ties broken by primary key.
- `traces_to_embed` correctly excludes rows that have an embedding for the active model and includes them after a model swap.
- Worker bulk-insert respects `ON CONFLICT (item_kind, item_id, model) DO NOTHING`.
- Orphan sweeper drops embeddings whose parent row has been cascade-deleted.

### 7.3 L3 — Integration (full daemon, mock provider)

- Start daemon with `MockEmbeddingProvider`; insert a session via ingest; assert the worker fills `embeddings` rows within 10 s; assert `vector_search_turns` returns the row when queried with a near-duplicate vector.
- `do_search` with `semantic_weight = 0.5`: embedding query failure path returns lexical-only results, sets `degraded = true`, never errors.
- Provider-swap test: write rows with `model="A"`, swap config to `model="B"`, restart, confirm the view re-queues all rows and the worker re-embeds without touching A-model rows.

### 7.4 L3 — Integration (real Ollama, GPU-preferred)

**Provider discovery order at test setup:**

1. Probe `http://localhost:11434/api/version` — if responsive AND the configured model is available locally, use that. This is the **preferred path on developer machines and any CI runner with Ollama pre-installed**; it uses whatever GPU the host has (Apple Silicon Metal, NVIDIA CUDA, ROCm).
2. Else: probe `ollama` binary on PATH. If present, run `ollama serve` in the background, `ollama pull <model>`, and use that. Local install still benefits from the host GPU.
3. Else (last resort, CI runners that lack Ollama): spawn a managed Docker sidecar with CPU-only image. Significantly slower; tagged separately so we can see the latency hit in test output.

Tagged tests `[ollama]`. CI matrix dimensions: `macos-arm64` (Metal GPU expected via local install), `ubuntu-22.04-with-ollama` (preinstall step), `ubuntu-22.04-fallback` (Docker, CPU only).

Tests at this layer:
- Embed real text via `nomic-embed-text`; assert dimension matches the trait's `dimension()`; assert `‖v‖ > 0`.
- End-to-end: insert a turn whose `user_prompt` paraphrases a target query, fill the embedding via the real worker, search with `semantic_weight = 0.6`, assert the paraphrased turn appears in the top-3 hits.
- Outage simulation: kill the Ollama process mid-batch, observe worker retry on next tick, confirm no data loss.

### 7.5 L4 — E2E with real Claude Code (nightly)

- Real Claude session producing ~10 turns; wait 30 s for the worker; query `teramind search "<paraphrase>"` with `semantic = 0.5`; assert at least one semantic-only hit (turn whose lexical content does not overlap the query but whose meaning does).

### 7.6 L5 — Search effectiveness benchmark

- `teramind-search-eval run --semantic` produces `eval-results-semantic.json` and `eval-scorecard-semantic.md`.
- `eval-semantic` CI job gates the semantic-on path against `baseline-semantic.json` (fail-soft until v1.0.1 corpus expansion).
- The lexical-only L5 gate (Plan F) stays exactly as it was — semantic eval cannot regress the existing gate.

### 7.7 Property-based and fault-injection tests

- For any provider, `embed(["x", "y"]).len() == 2` and each output has `provider.dimension()` floats.
- For any healthy-provider vector, `‖v‖ > 0`.
- `traces_to_embed` is a sound view: row appears iff no `embeddings` row exists for `(kind, id, active_model)`.
- Kill Ollama mid-batch → worker logs error, retries next tick, no data lost.
- Embed an oversized batch → provider returns 413 → worker bisects up to depth 4, then drops with `embedding_errors` increment.
- Bad model name → first health check fails → daemon stays up, worker pauses, `teramind doctor` shows actionable error.

### 7.8 Performance budgets

| Path | Budget |
|---|---|
| Vector top-K=10 on 10k vectors (cosine, HNSW) | p99 < 50 ms |
| Embedding worker throughput (`nomic-embed-text` on M1 GPU via Ollama) | ≥ 100 rows/s |
| Backfill of 10k-row corpus (worker default throughput) | < 100 s |
| Search blended (`semantic_weight = 0.4`) | p99 ≤ 1 s, target ≤ 800 ms |

## 8. Rollout, dependencies, risks

### 8.1 Dependencies

- Spec depends on Teramind Core (Plans A–F merged ✓).
- Does **not** depend on the other follow-on specs (session summarizer, skill codifier, team sync).
- Skill codifier (spec follow-on #3) gets cleaner clustering once pgvector lands — that's why we sequenced pgvector first.

### 8.2 Rollout phases

1. **v1.0 (this spec)** — schema, worker, Ollama default, fastembed fallback, semantic OFF by default, L5 `--semantic` eval as fail-soft instrumentation.
2. **v1.0.1** — corpus expansion: paraphrase queries in `queries_bank`, generator plants concept triggers, `eval-semantic` CI job flips to fail-hard.
3. **v1.1** — cloud providers (Anthropic, OpenAI, Voyage). Same trait surface; requires `network_egress = true`. Adds API-key handling and `max_embeddings_per_day` budget cap.

### 8.3 Open questions resolved during plan execution

- **`pgvector` version pin.** Migration uses `CREATE EXTENSION IF NOT EXISTS vector` without a version constraint. The `postgresql_embedded` builds we already ship include pgvector 0.7+; this is verified during plan task 1 (`migration applies; extension version >= 0.7`).
- **Token truncation strategy.** v1.0 uses a cheap byte-length heuristic (`bytes / 4 ≈ tokens`). Provider-specific tokenizers (where cheap to embed) are a v1.1 refinement.
- **Distance metric variation.** Cosine is the v1.0 default. The trait carries `distance_metric()` for future dot-product providers (Voyage); `SearchRepo` will branch on the metric in v1.1.

### 8.4 Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Ollama not installed on user's host | Medium | Clear `teramind init` message; auto-fallback to `FastEmbedProvider` if Ollama probe fails for 60s after init; `teramind doctor` flags the misconfig. |
| HNSW build cost at high write rate | Low | Measured ~10% insert overhead at 10k rows; acceptable. Index params (`m`, `ef_construction`) can be re-tuned in v1.1 once we have telemetry on a real >50k-row corpus. |
| Backfill thundering herd on upgrade | Medium | `max_throughput_per_min` config (default 1000) throttles the worker. `teramind doctor` reports backlog so users see progress. |
| Semantic regresses lexical metrics | Medium | Default-off ships zero behavior change. The `--semantic` eval mode + `baseline-semantic.json` separates the regression surfaces. Fail-soft CI prevents the gate from blocking work until the corpus reflects the use case. |
| pgvector missing from embedded PG bundle | Low | Migration fails with actionable error; daemon refuses to start; covered in L2 test. |
| Redaction gaps for embedding payloads | Low | Same `Redactor` as ingest persistence (no duplicate config); proptest property: no secret string appears in any post-redaction batch passed to a provider. |

### 8.5 Out of scope (deferred to later revisions / follow-on specs)

- Hybrid retrieval re-ranking (cross-encoder over top-K) — v2.
- Per-project model overrides (different model per `project_id`) — v1.1 if users ask.
- Embedding model fine-tuning — not Teramind's job.
- Real-user corpus contribution to L5 — already deferred in Plan F.

## 9. Glossary

- **Embedding** — a fixed-dimensional float vector that encodes a text's semantics. Two texts are "semantically similar" if the cosine distance between their embeddings is small.
- **Provider** — the source of embeddings. Local (Ollama/FastEmbed) or cloud (Anthropic/OpenAI/Voyage).
- **Model** — a specific embedding model within a provider, e.g. `nomic-embed-text` under Ollama.
- **Dimension** — vector length, tied to the model. `nomic-embed-text` = 768. Switching to a different-dim model requires a schema bump.
- **HNSW** — Hierarchical Navigable Small World, the pgvector index type used. Trades build time for query latency at scale.
- **Cosine distance** — pgvector's `<=>` operator. Bounded in `[0, 2]` for normalized vectors; we report `1 - distance` as `semantic_score` in `[0, 1]`.
- **Backlog** — count of rows in `traces_to_embed` that the worker hasn't filled yet for the active model. Surfaced via `teramind doctor`.
- **Provider swap** — changing `embed.toml.provider` or `embed.toml.model`. Triggers a re-embed loop without touching the old rows; rollback is "switch back and the old rows are still there."
- **Network egress** — outbound HTTPS to a non-localhost host. Refused unless the user opts in via `network_egress = true`.
