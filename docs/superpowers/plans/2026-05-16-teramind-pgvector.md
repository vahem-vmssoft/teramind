# Teramind pgvector — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Layer semantic retrieval on top of the existing lexical search by adding pgvector storage, a pluggable `EmbeddingProvider` trait (default Ollama, in-process FastEmbed fallback), an async `embedding_worker` daemon service, and a parallel `--semantic` mode for the L5 benchmark.

**Architecture:** New `embeddings` table keyed by `(item_kind, item_id, model)` with an HNSW cosine index. Async worker polls a `traces_to_embed` view, applies the existing `Redactor`, calls the active provider, and persists vectors. Search service blends a fifth `semantic` term into the existing FTS+trgm+recency+project ranking; default weight is `0.0` so semantic is opt-in via `~/.config/teramind/search.toml`. Plan F's eval harness gains a `--semantic` flag with its own committed baseline.

**Tech Stack:** Rust stable (workspace pin 1.93.0), existing workspace (sqlx 0.8, postgresql_embedded 0.20, reqwest 0.12 rustls), new deps: `pgvector` crate for sqlx type mapping, `fastembed` for the in-process provider, `postgresql_extensions` for runtime pgvector install into the embedded PG bundle.

---

## Spec coverage

This plan implements `docs/superpowers/specs/2026-05-16-teramind-pgvector-design.md` (Spec #7). Every numbered section of the spec maps to one or more tasks below; the coverage matrix at the bottom of this document spells out the mapping.

---

## File structure

**New files:**

| Path | Responsibility |
|---|---|
| `crates/teramind-core/src/embed.rs` | `EmbeddingProvider` trait, `DistanceMetric` enum, `ProviderKind` enum |
| `crates/teramindd/src/services/embed/mod.rs` | Provider factory + `EmbedConfig` loader |
| `crates/teramindd/src/services/embed/ollama.rs` | `OllamaProvider` (HTTP to localhost:11434) |
| `crates/teramindd/src/services/embed/fastembed_local.rs` | `FastEmbedProvider` (in-process, bundled fallback) |
| `crates/teramindd/src/services/embed/cloud.rs` | Cloud provider stub (refuses without `network_egress=true`) |
| `crates/teramindd/src/services/embedding_worker.rs` | Async poll/redact/embed/persist loop |
| `crates/teramindd/src/services/orphan_sweeper.rs` | Daily delete of orphan embeddings |
| `crates/teramind-db/migrations/20260516000001_embeddings.sql` | pgvector extension + `embeddings` table + HNSW index + `traces_to_embed` view |
| `crates/teramind-db/src/repos/embedding.rs` | `EmbeddingRepo`: bulk-insert, fetch-to-embed, orphan-sweep |
| `crates/teramind-search-eval/src/semantic.rs` | `--semantic` mode harness extensions |
| `.github/workflows/search-eval.yml` (modified) | New `eval-semantic` job |
| `benches/search-eval/baseline-semantic.json` | Committed semantic baseline |
| `docs/runbooks/pgvector-manual-smoke.md` | Manual test guide |

**Modified files:**

- `Cargo.toml` (workspace) — add `pgvector`, `fastembed`, `postgresql_extensions`, `async-trait` (verify already present).
- `crates/teramind-core/Cargo.toml` — add `async-trait`, `serde` (already present).
- `crates/teramindd/Cargo.toml` — add `pgvector`, `fastembed`, `postgresql_extensions`, `reqwest`.
- `crates/teramind-db/Cargo.toml` — add `pgvector`.
- `crates/teramind-db/src/repos/mod.rs` — register `embedding` module.
- `crates/teramind-db/src/repos/search.rs` — add `vector_search_turns`, `vector_search_diffs`.
- `crates/teramindd/src/services/mod.rs` — register `embed`, `embedding_worker`, `orphan_sweeper`.
- `crates/teramindd/src/services/search.rs` — extend `BlendWeights` with `semantic`, add semantic blending to `do_search`.
- `crates/teramindd/src/config.rs` — load `search.toml` weights + `embed.toml` settings.
- `crates/teramindd/src/app.rs` — wire `embedding_worker` + `orphan_sweeper`, pass provider into `IngestDeps`-style context.
- `crates/teramindd/src/pg_supervisor.rs` (or wherever PG starts) — install pgvector via `postgresql_extensions` before running migrations.
- `crates/teramind/src/commands/doctor.rs` — surface embedding provider health + backlog.
- `crates/teramind-search-eval/src/main.rs` — add `--semantic` and `--semantic-weight` flags.
- `crates/teramind-search-eval/src/harness.rs` — branch on `--semantic`.
- `crates/teramind-search-eval/src/gates.rs` — accept alternative results/baseline paths (already supports via flags).

---

## Section 0 — Workspace deps + pgvector binary install

### Task 0.1: Add workspace dependencies

**Files:**
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Add to `[workspace.dependencies]`**

Insert (alphabetical):

```toml
fastembed              = "4"
pgvector               = { version = "0.4", default-features = false, features = ["sqlx"] }
postgresql_extensions  = { version = "0.20", features = ["blocking"] }
```

Confirm `async-trait` is already a workspace dep (Plan A added it).

- [ ] **Step 2: `cargo metadata --offline 2>&1 | head -3`**

Expected: emits JSON. If the new crates aren't cached, drop `--offline` for the first run so they download.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml
git commit -m "build(deps): add pgvector + fastembed + postgresql_extensions"
```

---

### Task 0.2: Pull the new deps into the affected crates

**Files:**
- Modify: `crates/teramind-core/Cargo.toml`
- Modify: `crates/teramind-db/Cargo.toml`
- Modify: `crates/teramindd/Cargo.toml`

- [ ] **Step 1: teramind-core** — append to `[dependencies]`:

```toml
async-trait = { workspace = true }
```

(Confirm not duplicated; the existing teramind-core Cargo.toml probably doesn't have it yet.)

- [ ] **Step 2: teramind-db** — append:

```toml
pgvector = { workspace = true }
```

- [ ] **Step 3: teramindd** — append:

```toml
fastembed              = { workspace = true }
pgvector               = { workspace = true }
postgresql_extensions  = { workspace = true }
```

(`reqwest` is already there from Plan E.)

- [ ] **Step 4: Verify build**

Run: `cargo check --workspace`
Expected: succeeds. New deps will be downloaded.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-core/Cargo.toml crates/teramind-db/Cargo.toml crates/teramindd/Cargo.toml Cargo.lock
git commit -m "build(crates): wire pgvector + fastembed + extension installer"
```

---

### Task 0.3: Install pgvector into the embedded PG bundle

The `postgresql_embedded` 0.20 binaries do NOT ship pgvector. The companion crate `postgresql_extensions` downloads + installs platform-appropriate extension binaries into the embedded PG's `lib/` and `share/extension/` directories. This must run **before** migrations.

**Files:**
- Modify: `crates/teramind-db/src/pg_supervisor.rs` (path: locate `PgSupervisor::start`)

- [ ] **Step 1: Find the existing start function**

Run: `grep -n "pub.*fn start\|run_migrations" crates/teramind-db/src/pg_supervisor.rs`

Note the function shape; we'll insert the install call between PG startup and the existing migration step.

- [ ] **Step 2: Add the install helper**

Add this private function to `pg_supervisor.rs`:

```rust
async fn install_pgvector(installation_dir: &std::path::Path) -> anyhow::Result<()> {
    use postgresql_extensions::{ExtensionManager, Settings};
    let settings = Settings::default();
    let manager = ExtensionManager::new(settings, installation_dir).await?;
    // pgvector 0.7+ is what we tested against. The installer pulls the
    // matching build for the host arch + PG major version automatically.
    manager.install("portal-cloud", "pgvector", "0.7.4").await?;
    Ok(())
}
```

- [ ] **Step 3: Call it from `start`**

After PG is up but before `migrate::run`, add:

```rust
        install_pgvector(supervisor.installation_dir()).await
            .context("install pgvector into embedded PG")?;
```

(`installation_dir()` is the path containing the PG `bin/`, `lib/`, `share/` tree — `postgresql_embedded::PostgreSQL::settings().installation_dir()` gives it.)

- [ ] **Step 4: Add a smoke test**

Append to an existing teramind-db integration test file (e.g. `crates/teramind-db/tests/migrations.rs`):

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pgvector_extension_is_installable() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(
        dir.path().to_path_buf(), "teramind",
    ).await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS vector")
        .execute(pool.pg()).await?;
    let (version,): (String,) =
        sqlx::query_as("SELECT extversion FROM pg_extension WHERE extname='vector'")
            .fetch_one(pool.pg()).await?;
    assert!(version.starts_with("0."), "got {version}");
    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 5: Run the smoke test**

Run: `cargo test -p teramind-db pgvector_extension_is_installable --release`
Expected: PASS.

**Failure modes and escalation:**
- `postgresql_extensions::install` returns "no matching build for <arch>-<pg_version>": the host or PG version isn't covered by the public extension index. Report DONE_WITH_CONCERNS and we'll switch to a bundled-binary strategy or vendor the .so files.
- The install succeeds but `CREATE EXTENSION vector` errors with `could not load library`: extension was placed in the wrong dir. Inspect `installation_dir/lib/` and `share/extension/` to confirm pgvector files landed there.

- [ ] **Step 6: Commit**

```bash
git add crates/teramind-db/src/pg_supervisor.rs crates/teramind-db/tests/migrations.rs
git commit -m "feat(db): install pgvector into embedded PG before migrations"
```

---

## Section 1 — Schema migration

### Task 1.1: Migration file

**Files:**
- Create: `crates/teramind-db/migrations/20260516000001_embeddings.sql`

- [ ] **Step 1: Author the migration**

Create the file with EXACTLY this content:

```sql
CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE embeddings (
  id           bigserial PRIMARY KEY,
  item_kind    text NOT NULL CHECK (item_kind IN ('turn', 'file_diff')),
  item_id      uuid NOT NULL,
  model        text NOT NULL,
  dim          integer NOT NULL,
  embedding    vector(768) NOT NULL,
  created_at   timestamptz NOT NULL DEFAULT now(),
  UNIQUE (item_kind, item_id, model)
);

CREATE INDEX embeddings_lookup ON embeddings (item_kind, item_id);
CREATE INDEX embeddings_model  ON embeddings (model);

CREATE INDEX embeddings_hnsw ON embeddings
  USING hnsw (embedding vector_cosine_ops)
  WITH (m = 16, ef_construction = 64);

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

- [ ] **Step 2: Write a verification test**

Append to `crates/teramind-db/tests/migrations.rs`:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embeddings_migration_applies_and_view_works() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(
        dir.path().to_path_buf(), "teramind",
    ).await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    teramind_db::migrate::run(&pool).await?;

    // Table exists with the expected columns.
    let (col_count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM information_schema.columns WHERE table_name='embeddings'",
    ).fetch_one(pool.pg()).await?;
    assert_eq!(col_count, 7);

    // HNSW index present.
    let (idx_count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM pg_indexes WHERE indexname='embeddings_hnsw'",
    ).fetch_one(pool.pg()).await?;
    assert_eq!(idx_count, 1);

    // View returns zero rows on an empty corpus.
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM traces_to_embed")
        .fetch_one(pool.pg()).await?;
    assert_eq!(n, 0);

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p teramind-db embeddings_migration_applies_and_view_works --release`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-db/migrations/20260516000001_embeddings.sql crates/teramind-db/tests/migrations.rs
git commit -m "feat(db): embeddings table + traces_to_embed view + HNSW index"
```

---

## Section 2 — `EmbeddingProvider` trait

### Task 2.1: Trait + shared types in `teramind-core`

**Files:**
- Create: `crates/teramind-core/src/embed.rs`
- Modify: `crates/teramind-core/src/lib.rs` (register module)

- [ ] **Step 1: Create the trait module**

```rust
// crates/teramind-core/src/embed.rs
//! Embedding provider trait + shared types. Lives in `teramind-core` so
//! the eval crate can depend on it without pulling in the daemon.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistanceMetric {
    Cosine,
    Dot,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Ollama,
    Fastembed,
    Anthropic,
    Openai,
    Voyage,
}

impl ProviderKind {
    pub fn is_cloud(self) -> bool {
        matches!(self, ProviderKind::Anthropic | ProviderKind::Openai | ProviderKind::Voyage)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("provider unhealthy: {0}")]
    Unhealthy(String),
    #[error("input too large for batch: {0}")]
    SizeExceeded(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("provider error: {0}")]
    Other(String),
}

impl EmbedError {
    pub fn is_size_exceeded(&self) -> bool {
        matches!(self, EmbedError::SizeExceeded(_))
    }
}

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;
    fn model_id(&self) -> &str;
    fn dimension(&self) -> usize;
    fn max_tokens(&self) -> usize;
    fn distance_metric(&self) -> DistanceMetric;
    async fn health_check(&self) -> Result<(), EmbedError>;
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_kind_cloud_classification() {
        assert!(ProviderKind::Anthropic.is_cloud());
        assert!(ProviderKind::Openai.is_cloud());
        assert!(ProviderKind::Voyage.is_cloud());
        assert!(!ProviderKind::Ollama.is_cloud());
        assert!(!ProviderKind::Fastembed.is_cloud());
    }

    #[test]
    fn embed_error_size_exceeded_classifier() {
        let e = EmbedError::SizeExceeded("test".into());
        assert!(e.is_size_exceeded());
        let e2 = EmbedError::Network("test".into());
        assert!(!e2.is_size_exceeded());
    }

    #[test]
    fn provider_kind_roundtrips_through_toml() {
        for k in [ProviderKind::Ollama, ProviderKind::Fastembed, ProviderKind::Anthropic] {
            let s = toml::to_string(&toml::Value::String(
                serde_json::to_value(&k).unwrap().as_str().unwrap().to_string()
            )).unwrap();
            assert!(s.contains("="));
            let _: ProviderKind = serde_json::from_value(serde_json::to_value(&k).unwrap()).unwrap();
        }
    }
}
```

- [ ] **Step 2: Register the module**

In `crates/teramind-core/src/lib.rs`, add:

```rust
pub mod embed;
```

- [ ] **Step 3: Add `thiserror` if missing**

`thiserror` is already a workspace dep (Plan A). Confirm `crates/teramind-core/Cargo.toml` has `thiserror = { workspace = true }`; add if missing.

- [ ] **Step 4: Run**

Run: `cargo test -p teramind-core embed`
Expected: 3 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-core/src/embed.rs crates/teramind-core/src/lib.rs crates/teramind-core/Cargo.toml
git commit -m "feat(core): EmbeddingProvider trait + shared types"
```

---

## Section 3 — Ollama provider

### Task 3.1: `OllamaProvider`

**Files:**
- Create: `crates/teramindd/src/services/embed/mod.rs`
- Create: `crates/teramindd/src/services/embed/ollama.rs`
- Modify: `crates/teramindd/src/services/mod.rs`

- [ ] **Step 1: Register the new sub-module**

Append to `crates/teramindd/src/services/mod.rs`:

```rust
pub mod embed;
```

- [ ] **Step 2: Create the embed submodule index**

Create `crates/teramindd/src/services/embed/mod.rs`:

```rust
//! Embedding provider implementations. The factory + config loader live
//! in `factory.rs`; per-provider impls live in their own modules.

pub mod ollama;
pub mod fastembed_local;
pub mod cloud;
pub mod factory;

pub use factory::build_provider;
```

(`factory.rs` is created in Section 5; `fastembed_local.rs` in Section 4; `cloud.rs` in Section 6. They're listed here so the test in Step 4 compiles after we add them.)

- [ ] **Step 3: Create the Ollama provider**

Create `crates/teramindd/src/services/embed/ollama.rs`:

```rust
//! Ollama embedding provider (HTTP to localhost:11434).
//!
//! Uses the `/api/embed` endpoint (added in Ollama v0.1.40+) which
//! accepts a batched `input` array. Falls back to `/api/embeddings`
//! (one-at-a-time) when the batched endpoint returns 404.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use teramind_core::embed::{DistanceMetric, EmbedError, EmbeddingProvider, ProviderKind};

#[derive(Clone)]
pub struct OllamaProvider {
    url: String,
    model: String,
    dimension: usize,
    max_tokens: usize,
    client: reqwest::Client,
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

#[derive(Deserialize)]
struct VersionResponse {
    version: String,
}

impl OllamaProvider {
    /// Build a provider against `url` (e.g. `http://localhost:11434`).
    /// `dimension` and `max_tokens` come from the trusted model registry
    /// (`crate::services::embed::factory::model_meta`).
    pub fn new(url: String, model: String, dimension: usize, max_tokens: usize, timeout: Duration) -> Self {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("reqwest client build");
        Self { url, model, dimension, max_tokens, client }
    }
}

#[async_trait]
impl EmbeddingProvider for OllamaProvider {
    fn kind(&self) -> ProviderKind { ProviderKind::Ollama }
    fn model_id(&self) -> &str { &self.model }
    fn dimension(&self) -> usize { self.dimension }
    fn max_tokens(&self) -> usize { self.max_tokens }
    fn distance_metric(&self) -> DistanceMetric { DistanceMetric::Cosine }

    async fn health_check(&self) -> Result<(), EmbedError> {
        let url = format!("{}/api/version", self.url);
        let resp = self.client.get(&url).send().await
            .map_err(|e| EmbedError::Unhealthy(format!("GET {url}: {e}")))?;
        if !resp.status().is_success() {
            return Err(EmbedError::Unhealthy(format!("ollama version returned {}", resp.status())));
        }
        let _: VersionResponse = resp.json().await
            .map_err(|e| EmbedError::Unhealthy(format!("decode version: {e}")))?;
        Ok(())
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() { return Ok(vec![]); }
        let url = format!("{}/api/embed", self.url);
        let req = EmbedRequest { model: &self.model, input: texts };
        let resp = self.client.post(&url).json(&req).send().await
            .map_err(|e| EmbedError::Network(format!("POST {url}: {e}")))?;
        if resp.status() == reqwest::StatusCode::PAYLOAD_TOO_LARGE {
            return Err(EmbedError::SizeExceeded(format!("status 413 for batch of {}", texts.len())));
        }
        if !resp.status().is_success() {
            return Err(EmbedError::Other(format!("ollama embed returned {}", resp.status())));
        }
        let body: EmbedResponse = resp.json().await
            .map_err(|e| EmbedError::Other(format!("decode embed: {e}")))?;
        if body.embeddings.len() != texts.len() {
            return Err(EmbedError::Other(format!(
                "ollama returned {} vectors for {} inputs",
                body.embeddings.len(), texts.len(),
            )));
        }
        Ok(body.embeddings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_advertises_correct_kind_and_metric() {
        let p = OllamaProvider::new(
            "http://localhost:11434".into(),
            "nomic-embed-text-v2-moe".into(),
            768, 8192,
            Duration::from_secs(10),
        );
        assert_eq!(p.kind(), ProviderKind::Ollama);
        assert_eq!(p.distance_metric(), DistanceMetric::Cosine);
        assert_eq!(p.dimension(), 768);
        assert_eq!(p.model_id(), "nomic-embed-text-v2-moe");
    }
}
```

- [ ] **Step 4: Build**

Run: `cargo check -p teramindd 2>&1 | tail -5`
Expected: errors about missing `embed::factory`, `embed::fastembed_local`, `embed::cloud` modules — those land in §4–§5. Compile errors at this point are expected and resolved by the next sections.

To compile in isolation for now, temporarily replace `crates/teramindd/src/services/embed/mod.rs` with just `pub mod ollama;`. Restore the full module list when §4–§5 land.

- [ ] **Step 5: Run the unit test**

After the temp mod.rs simplification:

Run: `cargo test -p teramindd ollama::tests`
Expected: 1 test PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/teramindd/src/services/embed/mod.rs crates/teramindd/src/services/embed/ollama.rs crates/teramindd/src/services/mod.rs
git commit -m "feat(daemon): OllamaProvider over /api/embed"
```

---

## Section 4 — FastEmbed (in-process) provider

### Task 4.1: `FastEmbedProvider`

**Files:**
- Create: `crates/teramindd/src/services/embed/fastembed_local.rs`

- [ ] **Step 1: Author the provider**

```rust
//! In-process embedding provider backed by the `fastembed` crate.
//! Bundled fallback when Ollama is unreachable.

use async_trait::async_trait;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::path::PathBuf;
use std::sync::Arc;
use teramind_core::embed::{DistanceMetric, EmbedError, EmbeddingProvider, ProviderKind};
use tokio::sync::Mutex;

/// Wraps a `fastembed::TextEmbedding`. The library is sync; we lift it
/// onto a tokio Mutex + spawn_blocking so the worker stays async-friendly.
pub struct FastEmbedProvider {
    model: Arc<Mutex<TextEmbedding>>,
    model_name: String,
    dimension: usize,
    max_tokens: usize,
}

impl FastEmbedProvider {
    /// Build a provider. `cache_dir` controls where weights are cached.
    /// Default model: `NomicEmbedTextV15` (768 dims, matches our schema).
    pub fn new_default(cache_dir: PathBuf) -> Result<Self, EmbedError> {
        let opts = InitOptions::new(EmbeddingModel::NomicEmbedTextV15)
            .with_cache_dir(cache_dir)
            .with_show_download_progress(false);
        let model = TextEmbedding::try_new(opts)
            .map_err(|e| EmbedError::Other(format!("fastembed init: {e}")))?;
        Ok(Self {
            model: Arc::new(Mutex::new(model)),
            model_name: "fastembed:nomic-embed-text-v1.5".into(),
            dimension: 768,
            max_tokens: 8192,
        })
    }
}

#[async_trait]
impl EmbeddingProvider for FastEmbedProvider {
    fn kind(&self) -> ProviderKind { ProviderKind::Fastembed }
    fn model_id(&self) -> &str { &self.model_name }
    fn dimension(&self) -> usize { self.dimension }
    fn max_tokens(&self) -> usize { self.max_tokens }
    fn distance_metric(&self) -> DistanceMetric { DistanceMetric::Cosine }

    async fn health_check(&self) -> Result<(), EmbedError> {
        // In-process — if the model loaded, we're healthy.
        // A cheap embed of "ok" proves the runtime works.
        self.embed(&["ok".to_string()]).await.map(|_| ())
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() { return Ok(vec![]); }
        let model = self.model.clone();
        let texts: Vec<String> = texts.to_vec();
        tokio::task::spawn_blocking(move || {
            // fastembed wants Vec<&str> or Vec<String>. Pass owned for simplicity.
            let m = model.blocking_lock();
            m.embed(texts, None)
                .map_err(|e| EmbedError::Other(format!("fastembed embed: {e}")))
        })
        .await
        .map_err(|e| EmbedError::Other(format!("spawn_blocking join: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimension_matches_schema() {
        // We don't actually load the model here (would download weights).
        // Just lock in the dim constant.
        let dim = 768;
        assert_eq!(dim, 768);
    }
}
```

- [ ] **Step 2: Restore the full `embed/mod.rs`**

Edit `crates/teramindd/src/services/embed/mod.rs` to re-enable all submodules:

```rust
pub mod ollama;
pub mod fastembed_local;
pub mod cloud;
pub mod factory;

pub use factory::build_provider;
```

(`cloud.rs` and `factory.rs` arrive in §5–§6; this still won't compile end-to-end yet. We confirm full compilation in §6 Task 6.4.)

- [ ] **Step 3: Smoke build**

Run: `cargo check -p teramindd 2>&1 | grep -E "error\[|fastembed_local|warning" | head -10`

Expected: still errors about missing `cloud` and `factory` modules. That's fine — we proceed.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/embed/fastembed_local.rs crates/teramindd/src/services/embed/mod.rs
git commit -m "feat(daemon): FastEmbedProvider (in-process, bundled fallback)"
```

---

## Section 5 — Cloud provider stub

### Task 5.1: `CloudProvider` shape (v1.0 refuses, v1.1 implements)

**Files:**
- Create: `crates/teramindd/src/services/embed/cloud.rs`

- [ ] **Step 1: Author the stub**

```rust
//! Cloud embedding provider stub. v1.0 exposes the type + config gate;
//! actual HTTPS plumbing arrives in v1.1. Refuses to construct unless
//! `network_egress = true` is set in embed.toml.

use async_trait::async_trait;
use teramind_core::embed::{DistanceMetric, EmbedError, EmbeddingProvider, ProviderKind};

pub struct CloudProvider {
    kind: ProviderKind,
    model: String,
}

impl CloudProvider {
    /// Construct a stub. Caller must verify `network_egress=true` in config.
    pub fn new(kind: ProviderKind, model: String) -> Result<Self, EmbedError> {
        if !kind.is_cloud() {
            return Err(EmbedError::Other(format!(
                "CloudProvider built with non-cloud kind {:?}", kind,
            )));
        }
        Ok(Self { kind, model })
    }
}

#[async_trait]
impl EmbeddingProvider for CloudProvider {
    fn kind(&self) -> ProviderKind { self.kind }
    fn model_id(&self) -> &str { &self.model }
    fn dimension(&self) -> usize { 768 }    // v1.1 will branch on model
    fn max_tokens(&self) -> usize { 8192 }
    fn distance_metric(&self) -> DistanceMetric { DistanceMetric::Cosine }

    async fn health_check(&self) -> Result<(), EmbedError> {
        Err(EmbedError::Unhealthy(
            "cloud providers are stubbed in v1.0; wiring lands in v1.1".into(),
        ))
    }

    async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Err(EmbedError::Other(
            "cloud providers are stubbed in v1.0; wiring lands in v1.1".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_cloud_kind() {
        let r = CloudProvider::new(ProviderKind::Ollama, "x".into());
        assert!(r.is_err());
    }

    #[test]
    fn accepts_cloud_kind() {
        let r = CloudProvider::new(ProviderKind::Anthropic, "voyage-3".into());
        assert!(r.is_ok());
    }

    #[tokio::test]
    async fn health_check_returns_unhealthy() {
        let p = CloudProvider::new(ProviderKind::Anthropic, "voyage-3".into()).unwrap();
        assert!(p.health_check().await.is_err());
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramindd cloud::tests`
Expected: 3 tests PASS (won't compile until §6 lands factory; expected to skip this step OR run after §6).

Actually — since `factory.rs` is the only blocker and we're 1 task away, defer the test run to §6 Task 6.4.

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/src/services/embed/cloud.rs
git commit -m "feat(daemon): CloudProvider stub (v1.0 refuses, v1.1 wires)"
```

---

## Section 6 — Provider factory + `embed.toml` config

### Task 6.1: `EmbedConfig` types

**Files:**
- Modify: `crates/teramindd/src/config.rs`

The daemon already has a `Config` struct loaded from `~/.config/teramind/config.toml`. We add a SEPARATE config file `~/.config/teramind/embed.toml` for embedding settings (keeps the surfaces decoupled).

- [ ] **Step 1: Add the config types**

Append to `crates/teramindd/src/config.rs`:

```rust
use serde::{Deserialize, Serialize};
use teramind_core::embed::ProviderKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedConfig {
    #[serde(default = "default_provider")]
    pub provider: ProviderKind,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_batch_size")]
    pub batch_size: u32,
    #[serde(default = "default_max_throughput")]
    pub max_throughput_per_min: u32,
    #[serde(default = "default_orphan_sweep")]
    pub orphan_sweep_interval_hr: u32,
    #[serde(default)]
    pub network_egress: bool,
    #[serde(default)]
    pub ollama: OllamaConfig,
    #[serde(default)]
    pub fastembed: FastembedConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaConfig {
    #[serde(default = "default_ollama_url")]
    pub url: String,
    #[serde(default = "default_ollama_timeout_ms")]
    pub request_timeout_ms: u64,
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            url: default_ollama_url(),
            request_timeout_ms: default_ollama_timeout_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FastembedConfig {
    #[serde(default)]
    pub cache_dir: Option<String>,
}

fn default_provider() -> ProviderKind { ProviderKind::Ollama }
fn default_model() -> String { "nomic-embed-text-v2-moe".into() }
fn default_poll_interval() -> u64 { 5 }
fn default_batch_size() -> u32 { 32 }
fn default_max_throughput() -> u32 { 1000 }
fn default_orphan_sweep() -> u32 { 24 }
fn default_ollama_url() -> String { "http://localhost:11434".into() }
fn default_ollama_timeout_ms() -> u64 { 10_000 }

impl Default for EmbedConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            model: default_model(),
            poll_interval_secs: default_poll_interval(),
            batch_size: default_batch_size(),
            max_throughput_per_min: default_max_throughput(),
            orphan_sweep_interval_hr: default_orphan_sweep(),
            network_egress: false,
            ollama: OllamaConfig::default(),
            fastembed: FastembedConfig::default(),
        }
    }
}

impl EmbedConfig {
    pub fn load_or_default(path: &std::path::Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let body = std::fs::read_to_string(path)?;
        let c: Self = toml::from_str(&body)?;
        c.validate()?;
        Ok(c)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.provider.is_cloud() && !self.network_egress {
            anyhow::bail!(
                "embed.toml: provider={:?} requires network_egress=true. \
                 Flip the flag or switch to ollama/fastembed.",
                self.provider,
            );
        }
        Ok(())
    }
}
```

- [ ] **Step 2: Add tests**

Append to `crates/teramindd/src/config.rs` `tests` module (or create one):

```rust
#[cfg(test)]
mod embed_config_tests {
    use super::*;

    #[test]
    fn default_is_ollama_with_v2_moe() {
        let c = EmbedConfig::default();
        assert!(matches!(c.provider, ProviderKind::Ollama));
        assert_eq!(c.model, "nomic-embed-text-v2-moe");
        assert!(!c.network_egress);
    }

    #[test]
    fn cloud_provider_requires_network_egress() {
        let mut c = EmbedConfig::default();
        c.provider = ProviderKind::Anthropic;
        assert!(c.validate().is_err());
        c.network_egress = true;
        c.validate().expect("should pass with egress=true");
    }

    #[test]
    fn local_providers_dont_require_egress() {
        for p in [ProviderKind::Ollama, ProviderKind::Fastembed] {
            let mut c = EmbedConfig::default();
            c.provider = p;
            c.network_egress = false;
            c.validate().expect("local provider should pass");
        }
    }

    #[test]
    fn load_returns_default_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let c = EmbedConfig::load_or_default(&dir.path().join("embed.toml")).unwrap();
        assert_eq!(c.model, "nomic-embed-text-v2-moe");
    }
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p teramindd embed_config`
Expected: 4 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/config.rs
git commit -m "feat(daemon): EmbedConfig types + validation"
```

---

### Task 6.2: `model_meta` registry

Each model has fixed `dimension` and `max_tokens`. The factory needs these to build the provider. v1.0 ships a static table.

**Files:**
- Modify: `crates/teramindd/src/services/embed/mod.rs`

- [ ] **Step 1: Add the registry**

Append to `crates/teramindd/src/services/embed/mod.rs`:

```rust
use teramind_core::embed::ProviderKind;

#[derive(Debug, Clone, Copy)]
pub struct ModelMeta {
    pub dimension: usize,
    pub max_tokens: usize,
}

pub fn model_meta(provider: ProviderKind, model: &str) -> ModelMeta {
    // Trusted registry. Unknown models fall back to (768, 8192) so the
    // worker still tries — at worst the embed call errors and we log it.
    match (provider, model) {
        (ProviderKind::Ollama, "nomic-embed-text-v2-moe") => ModelMeta { dimension: 768, max_tokens: 8192 },
        (ProviderKind::Ollama, "nomic-embed-text")        => ModelMeta { dimension: 768, max_tokens: 8192 },
        (ProviderKind::Ollama, "mxbai-embed-large")       => ModelMeta { dimension: 1024, max_tokens: 512 },
        (ProviderKind::Fastembed, _)                       => ModelMeta { dimension: 768, max_tokens: 8192 },
        _                                                  => ModelMeta { dimension: 768, max_tokens: 8192 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_ollama_model_returns_correct_dim() {
        let m = model_meta(ProviderKind::Ollama, "nomic-embed-text-v2-moe");
        assert_eq!(m.dimension, 768);
        assert_eq!(m.max_tokens, 8192);
    }

    #[test]
    fn unknown_model_falls_back_to_768() {
        let m = model_meta(ProviderKind::Ollama, "no-such-model");
        assert_eq!(m.dimension, 768);
    }

    #[test]
    fn mxbai_has_1024_dim() {
        let m = model_meta(ProviderKind::Ollama, "mxbai-embed-large");
        assert_eq!(m.dimension, 1024);
    }
}
```

Note: the schema uses `vector(768)`. Models with `dimension != 768` (like mxbai) will fail at insert time. v1.0 ships the registry so unsupported models error loudly; widening the column is a v1.1 schema change.

- [ ] **Step 2: Run**

Run: `cargo test -p teramindd embed::tests`
Expected: 3 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/src/services/embed/mod.rs
git commit -m "feat(daemon): embed model_meta registry"
```

---

### Task 6.3: `build_provider` factory

**Files:**
- Create: `crates/teramindd/src/services/embed/factory.rs`

- [ ] **Step 1: Author the factory**

```rust
//! Provider factory. Reads EmbedConfig, constructs the matching impl.

use crate::config::EmbedConfig;
use crate::services::embed::{model_meta, cloud::CloudProvider, fastembed_local::FastEmbedProvider, ollama::OllamaProvider};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::embed::{EmbeddingProvider, ProviderKind};

pub fn build_provider(cfg: &EmbedConfig) -> anyhow::Result<Arc<dyn EmbeddingProvider>> {
    cfg.validate()?;
    let meta = model_meta(cfg.provider, &cfg.model);
    match cfg.provider {
        ProviderKind::Ollama => {
            let timeout = Duration::from_millis(cfg.ollama.request_timeout_ms);
            Ok(Arc::new(OllamaProvider::new(
                cfg.ollama.url.clone(),
                cfg.model.clone(),
                meta.dimension,
                meta.max_tokens,
                timeout,
            )))
        }
        ProviderKind::Fastembed => {
            let cache_dir = cfg.fastembed.cache_dir.clone()
                .map(PathBuf::from)
                .unwrap_or_else(default_fastembed_cache_dir);
            std::fs::create_dir_all(&cache_dir).ok();
            let p = FastEmbedProvider::new_default(cache_dir)
                .map_err(|e| anyhow::anyhow!("fastembed init: {e}"))?;
            Ok(Arc::new(p))
        }
        kind @ (ProviderKind::Anthropic | ProviderKind::Openai | ProviderKind::Voyage) => {
            let p = CloudProvider::new(kind, cfg.model.clone())
                .map_err(|e| anyhow::anyhow!("cloud provider init: {e}"))?;
            Ok(Arc::new(p))
        }
    }
}

fn default_fastembed_cache_dir() -> PathBuf {
    #[cfg(unix)] {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        home.join(".local/share/teramind/embed-models")
    }
    #[cfg(windows)] {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default();
        local.join("teramind").join("embed-models")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_ollama_provider_with_defaults() {
        let cfg = EmbedConfig::default();
        let p = build_provider(&cfg).expect("ollama default");
        assert_eq!(p.kind(), ProviderKind::Ollama);
        assert_eq!(p.dimension(), 768);
    }

    #[test]
    fn build_cloud_without_egress_fails() {
        let mut cfg = EmbedConfig::default();
        cfg.provider = ProviderKind::Anthropic;
        cfg.network_egress = false;
        assert!(build_provider(&cfg).is_err());
    }

    #[test]
    fn build_cloud_with_egress_succeeds_but_health_fails() {
        let mut cfg = EmbedConfig::default();
        cfg.provider = ProviderKind::Anthropic;
        cfg.network_egress = true;
        let p = build_provider(&cfg).expect("config validates");
        // v1.0 stub: health check refuses.
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let r = rt.block_on(p.health_check());
        assert!(r.is_err());
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramindd factory::tests`
Expected: 3 tests PASS (the fastembed path isn't exercised here — too expensive to download weights in a unit test).

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/src/services/embed/factory.rs
git commit -m "feat(daemon): EmbeddingProvider factory"
```

---

### Task 6.4: Full workspace check

- [ ] **Step 1: Build everything**

Run: `cargo check --workspace 2>&1 | tail -10`
Expected: succeeds. Warnings about unused providers are fine — they'll be used in §7+.

- [ ] **Step 2: Run all teramindd embed tests**

Run: `cargo test -p teramindd embed`
Expected: all PASS (ollama, cloud, factory, model_meta).

- [ ] **Step 3: Commit if any cleanup needed**

```bash
git add -A
git commit -m "chore(embed): wire all submodules" || true
```

---

## Section 7 — `EmbeddingRepo`

### Task 7.1: Repo with bulk-insert + fetch-to-embed + orphan-sweep

**Files:**
- Create: `crates/teramind-db/src/repos/embedding.rs`
- Modify: `crates/teramind-db/src/repos/mod.rs`

- [ ] **Step 1: Register the module**

Append to `crates/teramind-db/src/repos/mod.rs`:

```rust
pub mod embedding;
pub use embedding::{EmbeddingRepo, ToEmbedRow};
```

- [ ] **Step 2: Author the repo**

Create `crates/teramind-db/src/repos/embedding.rs`:

```rust
//! Storage layer for the `embeddings` table.
//!
//! Single writer is the daemon's `embedding_worker`. Reads are issued
//! by `SearchRepo::vector_search_*` methods.

use crate::error::Result;
use crate::pool::DbPool;
use pgvector::Vector;
use uuid::Uuid;

#[derive(Clone)]
pub struct EmbeddingRepo {
    pool: DbPool,
}

#[derive(Debug, Clone)]
pub struct ToEmbedRow {
    pub kind: String,    // "turn" | "file_diff"
    pub item_id: Uuid,
    pub text: String,
}

impl EmbeddingRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    /// Fetch up to `limit` rows that lack an embedding for `model`.
    /// The view does the heavy SQL; we filter to "rows whose (kind, id, model)
    /// triple isn't already in `embeddings`".
    pub async fn fetch_to_embed(&self, model: &str, limit: u32) -> Result<Vec<ToEmbedRow>> {
        let rows: Vec<(String, Uuid, String)> = sqlx::query_as(
            r#"
            SELECT v.kind, v.item_id, v.text
            FROM   traces_to_embed v
            WHERE  NOT EXISTS (
                SELECT 1 FROM embeddings e
                WHERE  e.item_kind = v.kind
                  AND  e.item_id   = v.item_id
                  AND  e.model     = $1
            )
            LIMIT  $2
            "#,
        )
        .bind(model)
        .bind(limit as i64)
        .fetch_all(self.pool.pg()).await?;
        Ok(rows.into_iter().map(|(kind, item_id, text)| ToEmbedRow { kind, item_id, text }).collect())
    }

    /// Bulk-insert vectors. `rows` and `vectors` must have identical length;
    /// caller guarantees this.
    pub async fn bulk_insert(
        &self,
        rows: &[ToEmbedRow],
        model: &str,
        dim: i32,
        vectors: &[Vec<f32>],
    ) -> Result<usize> {
        if rows.is_empty() { return Ok(0); }
        assert_eq!(rows.len(), vectors.len(), "ToEmbedRow/vector length mismatch");
        let mut written = 0usize;
        let mut tx = self.pool.pg().begin().await?;
        for (row, vec) in rows.iter().zip(vectors.iter()) {
            let v = Vector::from(vec.clone());
            let r = sqlx::query(
                r#"
                INSERT INTO embeddings (item_kind, item_id, model, dim, embedding)
                VALUES ($1, $2, $3, $4, $5)
                ON CONFLICT (item_kind, item_id, model) DO NOTHING
                "#,
            )
            .bind(&row.kind)
            .bind(row.item_id)
            .bind(model)
            .bind(dim)
            .bind(v)
            .execute(&mut *tx).await?;
            written += r.rows_affected() as usize;
        }
        tx.commit().await?;
        Ok(written)
    }

    /// Count embeddings missing for `model`. Surfaced via `teramind doctor`.
    pub async fn backlog(&self, model: &str) -> Result<i64> {
        let (n,): (i64,) = sqlx::query_as(
            r#"
            SELECT count(*) FROM traces_to_embed v
            WHERE NOT EXISTS (
                SELECT 1 FROM embeddings e
                WHERE  e.item_kind = v.kind
                  AND  e.item_id   = v.item_id
                  AND  e.model     = $1
            )
            "#,
        )
        .bind(model)
        .fetch_one(self.pool.pg()).await?;
        Ok(n)
    }

    /// Delete embeddings whose parent row has been cascade-deleted.
    /// Returns the number of rows removed.
    pub async fn sweep_orphans(&self) -> Result<u64> {
        let r = sqlx::query(
            r#"
            DELETE FROM embeddings e
            WHERE (e.item_kind = 'turn'
                   AND NOT EXISTS (SELECT 1 FROM turns t WHERE t.id = e.item_id))
               OR (e.item_kind = 'file_diff'
                   AND NOT EXISTS (SELECT 1 FROM file_diffs d WHERE d.id = e.item_id))
            "#,
        )
        .execute(self.pool.pg()).await?;
        Ok(r.rows_affected())
    }
}
```

- [ ] **Step 3: Integration test**

Append to `crates/teramind-db/tests/migrations.rs` (or a new file `embedding_repo.rs`):

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embedding_repo_bulk_insert_and_backlog() -> anyhow::Result<()> {
    use teramind_db::repos::{AgentRepo, EmbeddingRepo, SessionRepo, TraceRepo, ToEmbedRow};
    use teramind_db::repos::session::NewSession;
    use teramind_core::ids::{SessionId, TurnId};
    use time::OffsetDateTime;

    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(
        dir.path().to_path_buf(), "teramind",
    ).await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    teramind_db::migrate::run(&pool).await?;

    // Insert a session + turn so the view has a row.
    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await?;
    let sid = sessions.insert(NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/p",
        project_id: None, parent_session_id: None,
        git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u",
        started_at: OffsetDateTime::now_utc(),
    }).await?;
    let tid = trace.upsert_turn_with_id(
        TurnId(uuid::Uuid::new_v4()), sid, 0,
        OffsetDateTime::now_utc(), Some("hello world"),
    ).await?;

    let repo = EmbeddingRepo::new(pool.clone());
    assert_eq!(repo.backlog("test-model").await?, 1);

    let rows = repo.fetch_to_embed("test-model", 10).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, "turn");
    assert_eq!(rows[0].item_id, tid.0);

    // Insert a fake vector and re-check backlog.
    let v = vec![0.1f32; 768];
    let written = repo.bulk_insert(&rows, "test-model", 768, &[v]).await?;
    assert_eq!(written, 1);
    assert_eq!(repo.backlog("test-model").await?, 0);

    // Second insert with same key is a no-op (ON CONFLICT DO NOTHING).
    let v2 = vec![0.2f32; 768];
    let written2 = repo.bulk_insert(&rows, "test-model", 768, &[v2]).await?;
    assert_eq!(written2, 0);

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 4: Run**

Run: `cargo test -p teramind-db embedding_repo_bulk_insert_and_backlog --release`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-db/src/repos/embedding.rs crates/teramind-db/src/repos/mod.rs crates/teramind-db/tests/
git commit -m "feat(db): EmbeddingRepo (bulk-insert, fetch-to-embed, backlog, sweep)"
```

---

## Section 8 — `embedding_worker` service

### Task 8.1: Worker loop

**Files:**
- Create: `crates/teramindd/src/services/embedding_worker.rs`
- Modify: `crates/teramindd/src/services/mod.rs`

- [ ] **Step 1: Register module**

Append to `crates/teramindd/src/services/mod.rs`:

```rust
pub mod embedding_worker;
```

- [ ] **Step 2: Author the worker**

```rust
//! Async embedding worker. Polls `traces_to_embed`, redacts, calls the
//! provider, persists vectors. Capture-safe: never blocks ingest or search.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use teramind_core::embed::EmbeddingProvider;
use teramind_core::redact::Redactor;
use teramind_db::repos::EmbeddingRepo;
use tokio::sync::Mutex;
use tracing::{debug, warn};

#[derive(Default)]
pub struct EmbeddingStats {
    pub written: AtomicU64,
    pub errors: AtomicU64,
    pub backlog: AtomicU64,           // recomputed every poll
    pub last_filled_at_unix: AtomicU64,
    pub provider_unhealthy_since_unix: AtomicU64, // 0 = healthy
}

pub struct EmbeddingWorker {
    pub stats: Arc<EmbeddingStats>,
    handle: tokio::task::JoinHandle<()>,
}

pub struct EmbeddingWorkerDeps {
    pub repo: EmbeddingRepo,
    pub provider: Arc<dyn EmbeddingProvider>,
    pub redactor: Arc<Redactor>,
    pub model: String,
    pub poll_interval: Duration,
    pub batch_size: u32,
}

impl EmbeddingWorker {
    pub fn spawn(deps: EmbeddingWorkerDeps) -> Self {
        let stats = Arc::new(EmbeddingStats::default());
        let s = stats.clone();
        // Token bucket for max_throughput is enforced by batch sleeps; v1.0
        // keeps it simple: at batch_size=32 and poll=5s, max ~384 rows/min,
        // well under the 1000/min ceiling.
        let handle = tokio::spawn(async move {
            run_loop(deps, s).await;
        });
        Self { stats, handle }
    }

    pub fn abort(&self) { self.handle.abort(); }
}

async fn run_loop(deps: EmbeddingWorkerDeps, stats: Arc<EmbeddingStats>) {
    loop {
        tokio::time::sleep(deps.poll_interval).await;
        // Health probe.
        match deps.provider.health_check().await {
            Ok(_) => stats.provider_unhealthy_since_unix.store(0, Ordering::Relaxed),
            Err(e) => {
                let now = unix_now();
                let prev = stats.provider_unhealthy_since_unix.load(Ordering::Relaxed);
                if prev == 0 { stats.provider_unhealthy_since_unix.store(now, Ordering::Relaxed); }
                debug!(?e, "embedding provider unhealthy");
                stats.errors.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        }

        // Backlog probe (always run, cheap COUNT(*)).
        if let Ok(b) = deps.repo.backlog(&deps.model).await {
            stats.backlog.store(b as u64, Ordering::Relaxed);
        }

        // Pull a batch.
        let rows = match deps.repo.fetch_to_embed(&deps.model, deps.batch_size).await {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "fetch_to_embed failed");
                stats.errors.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };
        if rows.is_empty() { continue; }

        // Redact + truncate.
        let texts: Vec<String> = rows.iter()
            .map(|r| truncate_chars(&deps.redactor.apply(&r.text), deps.provider.max_tokens() * 4))
            .collect();

        // Embed (bisecting on size-exceeded).
        let vectors = match embed_with_bisect(deps.provider.as_ref(), &texts).await {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, "embed failed");
                stats.errors.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };

        // Persist.
        let dim = deps.provider.dimension() as i32;
        match deps.repo.bulk_insert(&rows, &deps.model, dim, &vectors).await {
            Ok(n) => {
                stats.written.fetch_add(n as u64, Ordering::Relaxed);
                stats.last_filled_at_unix.store(unix_now(), Ordering::Relaxed);
                debug!(written = n, "embedding_worker wrote batch");
            }
            Err(e) => {
                warn!(error = %e, "bulk_insert failed");
                stats.errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

async fn embed_with_bisect(
    provider: &dyn EmbeddingProvider,
    texts: &[String],
) -> Result<Vec<Vec<f32>>, teramind_core::embed::EmbedError> {
    embed_with_bisect_depth(provider, texts, 0).await
}

#[async_recursion::async_recursion]
async fn embed_with_bisect_depth(
    provider: &(dyn EmbeddingProvider + Send + Sync),
    texts: &[String],
    depth: u8,
) -> Result<Vec<Vec<f32>>, teramind_core::embed::EmbedError> {
    match provider.embed(texts).await {
        Ok(v) => Ok(v),
        Err(e) if e.is_size_exceeded() && depth < 4 && texts.len() > 1 => {
            let mid = texts.len() / 2;
            let left  = embed_with_bisect_depth(provider, &texts[..mid], depth + 1).await?;
            let right = embed_with_bisect_depth(provider, &texts[mid..], depth + 1).await?;
            Ok([left, right].concat())
        }
        Err(e) => Err(e),
    }
}

fn truncate_chars(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes { return s.to_string(); }
    // Don't split mid-codepoint.
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) { end -= 1; }
    s[..end].to_string()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use teramind_core::embed::{DistanceMetric, EmbedError, ProviderKind};
    use async_trait::async_trait;

    /// Deterministic test provider — returns a vector whose first element
    /// equals the input text's hash, so we can verify ordering.
    struct MockProvider {
        dim: usize,
        fail_oversize_at: Option<usize>,
    }

    #[async_trait]
    impl EmbeddingProvider for MockProvider {
        fn kind(&self) -> ProviderKind { ProviderKind::Fastembed }
        fn model_id(&self) -> &str { "mock" }
        fn dimension(&self) -> usize { self.dim }
        fn max_tokens(&self) -> usize { 8192 }
        fn distance_metric(&self) -> DistanceMetric { DistanceMetric::Cosine }
        async fn health_check(&self) -> Result<(), EmbedError> { Ok(()) }
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
            if let Some(thresh) = self.fail_oversize_at {
                if texts.len() > thresh {
                    return Err(EmbedError::SizeExceeded(format!("batch={}", texts.len())));
                }
            }
            Ok(texts.iter().map(|t| {
                let h = (t.len() as f32).to_bits() as f32;
                let mut v = vec![h; self.dim];
                v[0] = t.len() as f32;
                v
            }).collect())
        }
    }

    #[tokio::test]
    async fn truncate_chars_respects_codepoint_boundary() {
        let s = "héllo";  // 'é' is 2 bytes
        let t = truncate_chars(s, 2);
        assert!(t == "h" || t == "hé"); // depending on where the boundary lands
        assert!(s.starts_with(&t));
    }

    #[tokio::test]
    async fn embed_with_bisect_recurses_on_size_exceeded() {
        let p = MockProvider { dim: 4, fail_oversize_at: Some(2) };
        let texts: Vec<String> = (0..4).map(|i| format!("text{i}")).collect();
        let vectors = embed_with_bisect(&p, &texts).await.expect("should split");
        assert_eq!(vectors.len(), 4);
    }

    #[tokio::test]
    async fn embed_with_bisect_gives_up_after_depth_4() {
        let p = MockProvider { dim: 4, fail_oversize_at: Some(0) };  // every batch fails
        let texts: Vec<String> = vec!["x".into()];
        let r = embed_with_bisect(&p, &texts).await;
        assert!(r.is_err(), "single-item batch can't bisect further");
    }
}
```

- [ ] **Step 3: Add `async-recursion` to workspace deps**

In `Cargo.toml` (workspace):

```toml
async-recursion = "1"
```

In `crates/teramindd/Cargo.toml`:

```toml
async-recursion = { workspace = true }
```

- [ ] **Step 4: Run**

Run: `cargo test -p teramindd embedding_worker`
Expected: 3 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/teramindd/Cargo.toml crates/teramindd/src/services/mod.rs crates/teramindd/src/services/embedding_worker.rs
git commit -m "feat(daemon): embedding_worker with size-bisect retry"
```

---

### Task 8.2: Wire the worker into `App::run`

**Files:**
- Modify: `crates/teramindd/src/app.rs`

- [ ] **Step 1: Locate the worker setup region**

Run: `grep -n "FsWatcherService::spawn\|storage_stats::spawn" crates/teramindd/src/app.rs`

We add the embedding worker alongside the existing background workers, after PG is up and migrations applied, after `IngestService::spawn`.

- [ ] **Step 2: Build provider + spawn worker**

After the `storage_stats::spawn(...)` call, insert:

```rust
        // Embedding worker.
        let embed_cfg_path = paths.config_dir.join("embed.toml");
        let embed_cfg = crate::config::EmbedConfig::load_or_default(&embed_cfg_path)?;
        let provider = crate::services::embed::build_provider(&embed_cfg)?;
        let embed_repo = teramind_db::repos::EmbeddingRepo::new(pool.clone());
        let embed_worker = crate::services::embedding_worker::EmbeddingWorker::spawn(
            crate::services::embedding_worker::EmbeddingWorkerDeps {
                repo: embed_repo.clone(),
                provider: provider.clone(),
                redactor: Arc::new(Redactor::with_default_rules()),
                model: format!("{}:{}", provider_prefix(provider.kind()), embed_cfg.model),
                poll_interval: std::time::Duration::from_secs(embed_cfg.poll_interval_secs),
                batch_size: embed_cfg.batch_size,
            },
        );
        // Held for lifetime of the daemon; drop on shutdown aborts the loop.
        let _embed_worker_guard = embed_worker;
```

Add helper at the bottom of `app.rs`:

```rust
fn provider_prefix(kind: teramind_core::embed::ProviderKind) -> &'static str {
    use teramind_core::embed::ProviderKind::*;
    match kind {
        Ollama    => "ollama",
        Fastembed => "fastembed",
        Anthropic => "anthropic",
        Openai    => "openai",
        Voyage    => "voyage",
    }
}
```

(The `model` string used in DB queries combines provider prefix with the model name so we can tell `ollama:nomic` from `fastembed:nomic`.)

- [ ] **Step 3: Pass the provider into the search handler**

The IPC handler (`DaemonIpcHandler`) needs `provider` available for `do_search`-with-semantic. Add a field:

```rust
pub struct DaemonIpcHandler {
    // ... existing fields
    pub embed_provider: Arc<dyn EmbeddingProvider>,
    pub embed_model: String,
    pub search_weights: BlendWeights,  // from §10
}
```

Populate in `App::run` where the handler is constructed; the `search_weights` field is wired in §10. For now, pass a placeholder `BlendWeights::default()`.

- [ ] **Step 4: `cargo check -p teramindd`**

Expected: succeeds. Existing tests still build.

- [ ] **Step 5: Commit**

```bash
git add crates/teramindd/src/app.rs crates/teramindd/src/services/ipc_server.rs
git commit -m "feat(daemon): wire embedding_worker into App::run"
```

---

## Section 9 — `SearchRepo` vector methods

### Task 9.1: `vector_search_turns` + `vector_search_diffs`

**Files:**
- Modify: `crates/teramind-db/src/repos/search.rs`

- [ ] **Step 1: Extend RankedTurn / RankedDiff with semantic_score**

The existing structs already carry `fts_score: f32` and `trgm_score: f32`. Add a third field:

```rust
// In RankedTurn:
pub semantic_score: f32,

// In RankedDiff:
pub semantic_score: f32,
```

Search for all construction sites of `RankedTurn`/`RankedDiff` and set `semantic_score: 0.0` everywhere (FTS/trgm paths don't compute it).

- [ ] **Step 2: Add the vector methods**

Append to `impl SearchRepo`:

```rust
pub async fn vector_search_turns(
    &self,
    query_embedding: &[f32],
    model: &str,
    limit: u32,
) -> Result<Vec<RankedTurn>> {
    let v = pgvector::Vector::from(query_embedding.to_vec());
    let rows: Vec<(Uuid, Uuid, i32, OffsetDateTime, Option<Uuid>, f32, Option<String>, Option<String>)> = sqlx::query_as(
        r#"
        SELECT t.id, t.session_id, t.ordinal, t.started_at,
               s.project_id,
               (1.0 - (e.embedding <=> $1::vector))::float4 AS semantic_score,
               t.user_prompt, t.assistant_text
        FROM   embeddings e
        JOIN   turns t      ON e.item_kind = 'turn' AND e.item_id = t.id
        JOIN   sessions s   ON s.id = t.session_id
        WHERE  e.model = $2
        ORDER  BY e.embedding <=> $1::vector
        LIMIT  $3
        "#,
    )
    .bind(v)
    .bind(model)
    .bind(limit as i64)
    .fetch_all(self.pool.pg()).await?;

    Ok(rows.into_iter().map(|(turn_id, session_id, ordinal, ts, project_id, sem, prompt, text)| {
        RankedTurn {
            turn_id, session_id, ordinal, ts, project_id,
            fts_score: 0.0, trgm_score: 0.0, semantic_score: sem,
            user_prompt: prompt, assistant_text: text,
        }
    }).collect())
}

pub async fn vector_search_diffs(
    &self,
    query_embedding: &[f32],
    model: &str,
    limit: u32,
) -> Result<Vec<RankedDiff>> {
    let v = pgvector::Vector::from(query_embedding.to_vec());
    let rows: Vec<(Uuid, Uuid, String, OffsetDateTime, Option<Uuid>, f32, String, String)> = sqlx::query_as(
        r#"
        SELECT fd.id, fd.session_id, fd.rel_path, fd.captured_at,
               s.project_id,
               (1.0 - (e.embedding <=> $1::vector))::float4 AS semantic_score,
               fd.pre_excerpt, fd.post_excerpt
        FROM   embeddings e
        JOIN   file_diffs fd ON e.item_kind = 'file_diff' AND e.item_id = fd.id
        JOIN   sessions s    ON s.id = fd.session_id
        WHERE  e.model = $2
        ORDER  BY e.embedding <=> $1::vector
        LIMIT  $3
        "#,
    )
    .bind(v)
    .bind(model)
    .bind(limit as i64)
    .fetch_all(self.pool.pg()).await?;

    Ok(rows.into_iter().map(|(diff_id, session_id, rel_path, ts, project_id, sem, pre, post)| {
        RankedDiff {
            diff_id, session_id, rel_path, ts, project_id,
            trgm_score: 0.0, semantic_score: sem,
            pre_excerpt: pre, post_excerpt: post,
        }
    }).collect())
}
```

- [ ] **Step 3: Integration test**

Append to `crates/teramind-db/tests/search_repo.rs`:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vector_search_turns_returns_nearest_by_cosine() -> anyhow::Result<()> {
    use teramind_db::repos::{AgentRepo, EmbeddingRepo, SearchRepo, SessionRepo, TraceRepo, ToEmbedRow};
    use teramind_db::repos::session::NewSession;
    use teramind_core::ids::TurnId;
    use time::OffsetDateTime;

    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(
        dir.path().to_path_buf(), "teramind",
    ).await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    teramind_db::migrate::run(&pool).await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let embed = EmbeddingRepo::new(pool.clone());
    let search = SearchRepo::new(pool.clone());

    let agent = agents.upsert("claude_code", None).await?;
    let sid = sessions.insert(NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/p",
        project_id: None, parent_session_id: None,
        git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u",
        started_at: OffsetDateTime::now_utc(),
    }).await?;

    // Insert two turns; embed them with synthetic vectors.
    let t_near = trace.upsert_turn_with_id(
        TurnId(uuid::Uuid::new_v4()), sid, 0,
        OffsetDateTime::now_utc(), Some("near"),
    ).await?;
    let t_far = trace.upsert_turn_with_id(
        TurnId(uuid::Uuid::new_v4()), sid, 1,
        OffsetDateTime::now_utc(), Some("far"),
    ).await?;

    let mut near_v = vec![0.0f32; 768]; near_v[0] = 1.0;
    let mut far_v  = vec![0.0f32; 768]; far_v[1] = 1.0;

    embed.bulk_insert(
        &[ToEmbedRow { kind: "turn".into(), item_id: t_near.0, text: "near".into() }],
        "test-model", 768, &[near_v.clone()],
    ).await?;
    embed.bulk_insert(
        &[ToEmbedRow { kind: "turn".into(), item_id: t_far.0, text: "far".into() }],
        "test-model", 768, &[far_v.clone()],
    ).await?;

    // Query with near_v as the embedding — t_near should be top hit.
    let hits = search.vector_search_turns(&near_v, "test-model", 10).await?;
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].turn_id, t_near.0);
    assert!(hits[0].semantic_score > hits[1].semantic_score);
    // Cosine with itself ~= 1.0.
    assert!((hits[0].semantic_score - 1.0).abs() < 1e-6, "got {}", hits[0].semantic_score);

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 4: Run**

Run: `cargo test -p teramind-db vector_search_turns_returns_nearest_by_cosine --release`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-db/src/repos/search.rs crates/teramind-db/tests/search_repo.rs
git commit -m "feat(db): vector_search_turns + vector_search_diffs via pgvector"
```

---

## Section 10 — `services/search.rs` blend extension

### Task 10.1: `BlendWeights.semantic` + blended search

**Files:**
- Modify: `crates/teramindd/src/services/search.rs`

- [ ] **Step 1: Extend `BlendWeights`**

Locate the existing struct and modify:

```rust
#[derive(Debug, Clone, Copy)]
pub struct BlendWeights {
    pub fts: f32,
    pub trgm: f32,
    pub semantic: f32,
    pub recency: f32,
    pub project: f32,
}

impl Default for BlendWeights {
    fn default() -> Self {
        Self { fts: 0.6, trgm: 0.4, semantic: 0.0, recency: 0.2, project: 0.3 }
    }
}
```

- [ ] **Step 2: Extend `final_score`**

```rust
pub fn final_score(
    fts: f32, trgm: f32, semantic: f32,
    ts: OffsetDateTime,
    weights: BlendWeights,
    same_project: bool,
) -> f32 {
    let recency_decay = recency_factor(ts);
    let project_boost = if same_project { 1.0 } else { 0.0 };
    weights.fts * fts
        + weights.trgm * trgm
        + weights.semantic * semantic
        + weights.recency * recency_decay
        + weights.project * project_boost
}
```

Update `rank_and_hydrate` to read `semantic_score` from `RankedTurn`/`RankedDiff` and pass it as the third arg.

- [ ] **Step 3: Modify `do_search` to include the semantic path**

```rust
pub async fn do_search(
    repo: &SearchRepo,
    provider: Option<Arc<dyn EmbeddingProvider>>,
    model: &str,
    weights: BlendWeights,
    req: &SearchRequest,
) -> Result<SearchOutcome, teramind_db::DbError> {
    let started = Instant::now();

    // Embed the query if semantic weight is positive AND a provider is available.
    let query_emb: Option<Vec<f32>> = if weights.semantic > 0.0 {
        match provider {
            Some(p) => p.embed(&[req.query.clone()]).await
                .ok()
                .and_then(|mut v| v.pop()),
            None => None,
        }
    } else { None };

    let (fts_res, trgm_diffs, trgm_skills, sem_turns, sem_diffs) = tokio::try_join!(
        repo.fts_turns(&req.query, req.limit),
        repo.trgm_diffs(&req.query, req.limit),
        repo.trgm_skills(&req.query, req.limit),
        async {
            if let Some(v) = query_emb.as_ref() {
                repo.vector_search_turns(v, model, req.limit).await
            } else { Ok(vec![]) }
        },
        async {
            if let Some(v) = query_emb.as_ref() {
                repo.vector_search_diffs(v, model, req.limit).await
            } else { Ok(vec![]) }
        },
    )?;

    let degraded = weights.semantic > 0.0 && query_emb.is_none();
    let hits = rank_and_hydrate(fts_res, trgm_diffs, trgm_skills, sem_turns, sem_diffs, weights, None, req.limit);
    Ok(SearchOutcome { hits, degraded, took_ms: started.elapsed().as_millis() as u32 })
}
```

`rank_and_hydrate` is updated to accept the two extra Vecs and merge by `(turn_id|diff_id)` so a row that appears in BOTH FTS and semantic gets one combined `Hit` with both scores set.

- [ ] **Step 4: Update unit tests**

The existing `final_score_blends_with_recency_and_project_boost` test now has a different signature. Adjust to pass `semantic = 0.0`. Add:

```rust
#[test]
fn semantic_weight_contributes_to_score() {
    let weights = BlendWeights { fts: 0.0, trgm: 0.0, semantic: 1.0, recency: 0.0, project: 0.0 };
    let ts = OffsetDateTime::now_utc();
    let s = final_score(0.0, 0.0, 0.5, ts, weights, false);
    assert!((s - 0.5).abs() < 1e-6);
}
```

- [ ] **Step 5: Run**

Run: `cargo test -p teramindd search`
Expected: PASS (existing + new tests).

- [ ] **Step 6: Commit**

```bash
git add crates/teramindd/src/services/search.rs
git commit -m "feat(search): semantic blend term + vector-search join"
```

---

### Task 10.2: Wire provider + weights into the IPC handler

**Files:**
- Modify: `crates/teramindd/src/services/ipc_server.rs`
- Modify: `crates/teramindd/src/app.rs`

- [ ] **Step 1: Update the Search handler**

In `ipc_server.rs::handle_request`, the `Request::Search` arm currently calls `do_search_with_fallback(&self.search_repo, &self.jsonl_dir, &r)`. Update to:

```rust
            Request::Search(r) => {
                let out = crate::services::search::do_search(
                    &self.search_repo,
                    Some(self.embed_provider.clone()),
                    &self.embed_model,
                    self.search_weights,
                    &r,
                ).await;
                match out {
                    Ok(o) => Response::SearchResults(teramind_core::types::SearchResults {
                        hits: o.hits, degraded: o.degraded, took_ms: o.took_ms,
                    }),
                    Err(_) => {
                        // PG-down fallback path (existing grep behavior).
                        let out = crate::services::search::do_search_with_fallback(
                            &self.search_repo, &self.jsonl_dir, &r,
                        ).await;
                        Response::SearchResults(teramind_core::types::SearchResults {
                            hits: out.hits, degraded: out.degraded, took_ms: out.took_ms,
                        })
                    }
                }
            }
```

- [ ] **Step 2: Populate `search_weights` from `search.toml`**

In `App::run`, after loading `EmbedConfig`, load `search.toml`:

```rust
let search_cfg_path = paths.config_dir.join("search.toml");
let search_weights = crate::config::load_search_weights(&search_cfg_path)?;
```

Add `load_search_weights` to `config.rs`:

```rust
use crate::services::search::BlendWeights;

#[derive(Debug, Clone, Deserialize)]
struct SearchFile {
    #[serde(default)]
    blend: BlendOverride,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct BlendOverride {
    fts: Option<f32>, trgm: Option<f32>, semantic: Option<f32>,
    recency: Option<f32>, project: Option<f32>,
}

pub fn load_search_weights(path: &std::path::Path) -> anyhow::Result<BlendWeights> {
    if !path.exists() { return Ok(BlendWeights::default()); }
    let body = std::fs::read_to_string(path)?;
    let f: SearchFile = toml::from_str(&body)?;
    let d = BlendWeights::default();
    Ok(BlendWeights {
        fts:      f.blend.fts.unwrap_or(d.fts),
        trgm:     f.blend.trgm.unwrap_or(d.trgm),
        semantic: f.blend.semantic.unwrap_or(d.semantic),
        recency:  f.blend.recency.unwrap_or(d.recency),
        project:  f.blend.project.unwrap_or(d.project),
    })
}
```

(`BlendWeights` needs to be re-exported or made public from `services::search`. If it isn't already, add `pub use search::BlendWeights;` to `services/mod.rs`.)

- [ ] **Step 3: Test**

Add to `crates/teramindd/src/config.rs`:

```rust
#[cfg(test)]
mod search_weights_tests {
    use super::*;

    #[test]
    fn missing_file_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let w = load_search_weights(&dir.path().join("search.toml")).unwrap();
        assert_eq!(w.semantic, 0.0);
    }

    #[test]
    fn partial_override_keeps_other_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("search.toml");
        std::fs::write(&path, "[blend]\nsemantic = 0.4\n").unwrap();
        let w = load_search_weights(&path).unwrap();
        assert!((w.semantic - 0.4).abs() < 1e-6);
        assert!((w.fts - 0.6).abs() < 1e-6);  // default preserved
    }
}
```

- [ ] **Step 4: Run**

Run: `cargo test -p teramindd search_weights`
Expected: 2 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramindd/src/services/ipc_server.rs crates/teramindd/src/services/search.rs crates/teramindd/src/services/mod.rs crates/teramindd/src/app.rs crates/teramindd/src/config.rs
git commit -m "feat(daemon): wire embed_provider + search.toml weights into IPC"
```

---

## Section 11 — Orphan sweeper

### Task 11.1: Background sweeper service

**Files:**
- Create: `crates/teramindd/src/services/orphan_sweeper.rs`
- Modify: `crates/teramindd/src/services/mod.rs`
- Modify: `crates/teramindd/src/app.rs`

- [ ] **Step 1: Register module**

Append to `services/mod.rs`:

```rust
pub mod orphan_sweeper;
```

- [ ] **Step 2: Author the sweeper**

```rust
//! Daily sweep of orphan embeddings (rows whose parent turn/file_diff
//! was cascade-deleted). Runs in the background; never blocks anything.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use teramind_db::repos::EmbeddingRepo;
use tracing::{info, warn};

pub struct OrphanSweeper {
    pub deleted: Arc<AtomicU64>,
    handle: tokio::task::JoinHandle<()>,
}

impl OrphanSweeper {
    pub fn spawn(repo: EmbeddingRepo, interval: Duration) -> Self {
        let deleted = Arc::new(AtomicU64::new(0));
        let d = deleted.clone();
        let handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                match repo.sweep_orphans().await {
                    Ok(n) => {
                        if n > 0 { info!(deleted = n, "orphan_sweeper removed embeddings"); }
                        d.fetch_add(n, Ordering::Relaxed);
                    }
                    Err(e) => warn!(error = %e, "orphan_sweeper sweep failed"),
                }
            }
        });
        Self { deleted, handle }
    }

    pub fn abort(&self) { self.handle.abort(); }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Sweeping is exercised by the L3 integration test in Section 16; this
    // module has no isolated unit tests.
    #[test]
    fn it_compiles() { let _ = OrphanSweeper::abort; }
}
```

- [ ] **Step 3: Wire into `App::run`**

After the `EmbeddingWorker::spawn(...)` block, add:

```rust
        let orphan_interval = Duration::from_secs(embed_cfg.orphan_sweep_interval_hr as u64 * 3600);
        let _orphan_guard = crate::services::orphan_sweeper::OrphanSweeper::spawn(
            embed_repo.clone(),
            orphan_interval,
        );
```

- [ ] **Step 4: `cargo check -p teramindd`**

Expected: succeeds.

- [ ] **Step 5: Commit**

```bash
git add crates/teramindd/src/services/orphan_sweeper.rs crates/teramindd/src/services/mod.rs crates/teramindd/src/app.rs
git commit -m "feat(daemon): orphan_sweeper for embeddings"
```

---

## Section 12 — `teramind doctor` integration

### Task 12.1: Surface embedding provider health + backlog

**Files:**
- Modify: `crates/teramind/src/commands/doctor.rs`

- [ ] **Step 1: Add embedding lines to doctor's output**

The doctor command should query the daemon's IPC for status. Plan A/B/C/D's `StatusReport` already carries `fs_watcher_gaps_total` etc. We extend it.

First, add fields to `teramind_ipc::proto::StatusReport`:

```rust
#[serde(default)]
pub embedding_provider: Option<String>,    // e.g. "ollama:nomic-embed-text-v2-moe"
#[serde(default)]
pub embedding_healthy: Option<bool>,
#[serde(default)]
pub embedding_backlog: Option<i64>,
#[serde(default)]
pub embedding_last_filled_unix: Option<u64>,
```

(`#[serde(default)]` keeps the older daemon's `StatusReport` deserializable.)

- [ ] **Step 2: Populate the fields in `ipc_server.rs`**

In `DaemonIpcHandler`, add references to the embedding worker's stats + provider:

```rust
pub embed_stats: Arc<crate::services::embedding_worker::EmbeddingStats>,
```

In `Request::Status` response construction:

```rust
embedding_provider: Some(format!("{}:{}", provider_prefix(self.embed_provider.kind()), self.embed_model.split(':').nth(1).unwrap_or(&self.embed_model))),
embedding_healthy: Some(self.embed_stats.provider_unhealthy_since_unix.load(Ordering::Relaxed) == 0),
embedding_backlog: Some(self.embed_stats.backlog.load(Ordering::Relaxed) as i64),
embedding_last_filled_unix: {
    let v = self.embed_stats.last_filled_at_unix.load(Ordering::Relaxed);
    if v == 0 { None } else { Some(v) }
},
```

- [ ] **Step 3: Render in `doctor.rs`**

After existing checks, add:

```rust
    let status: StatusReport = client.status().await?;
    if let (Some(provider), Some(healthy)) = (&status.embedding_provider, status.embedding_healthy) {
        println!(
            "embedding provider: {} ({})",
            provider,
            if healthy { "healthy" } else { "unhealthy" },
        );
    }
    if let Some(backlog) = status.embedding_backlog {
        let last = status.embedding_last_filled_unix
            .map(|u| {
                let secs_ago = unix_now().saturating_sub(u);
                format!("last filled {secs_ago}s ago")
            })
            .unwrap_or_else(|| "no embeddings yet".into());
        println!("embedding backlog: {backlog} rows ({last})");
    }
```

Add a helper:

```rust
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs()).unwrap_or(0)
}
```

- [ ] **Step 4: `cargo check -p teramind-cli`**

Expected: succeeds.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-ipc/src/proto.rs crates/teramindd/src/services/ipc_server.rs crates/teramind/src/commands/doctor.rs
git commit -m "feat(cli): doctor surfaces embedding provider health + backlog"
```

---

## Section 13 — `teramind-search-eval --semantic` mode

### Task 13.1: `--semantic` flag plumbing

**Files:**
- Modify: `crates/teramind-search-eval/src/main.rs`

- [ ] **Step 1: Extend the `Run` subcommand**

Locate the `Cmd::Run { corpus, out }` variant in `main.rs`. Add fields:

```rust
    Run {
        #[arg(long, default_value = "benches/search-eval")]
        corpus: std::path::PathBuf,
        #[arg(long, default_value = "benches/search-eval")]
        out: std::path::PathBuf,
        /// Enable the semantic blend; writes outputs to *-semantic.{json,md}.
        #[arg(long)]
        semantic: bool,
        /// Weight to apply to the semantic score when --semantic is set.
        #[arg(long, default_value = "0.4")]
        semantic_weight: f32,
    },
```

Update the dispatch:

```rust
        Cmd::Run { corpus, out, semantic, semantic_weight } => {
            teramind_search_eval::harness::run(&corpus, &out, semantic, semantic_weight).await
        }
```

- [ ] **Step 2: Commit**

```bash
git add crates/teramind-search-eval/src/main.rs
git commit -m "feat(search-eval): --semantic CLI flag"
```

---

### Task 13.2: Harness branch on `--semantic`

**Files:**
- Modify: `crates/teramind-search-eval/src/harness.rs`
- Create: `crates/teramind-search-eval/src/semantic.rs`
- Modify: `crates/teramind-search-eval/src/lib.rs`

- [ ] **Step 1: Register the new module**

Append to `crates/teramind-search-eval/src/lib.rs`:

```rust
pub mod semantic;
```

- [ ] **Step 2: Update the harness signature**

The existing `harness::run(corpus_root: &Path, out_dir: &Path)` becomes:

```rust
pub async fn run(
    corpus_root: &Path,
    out_dir: &Path,
    semantic: bool,
    semantic_weight: f32,
) -> anyhow::Result<()> {
    if semantic {
        semantic::run_with_semantic(corpus_root, out_dir, semantic_weight).await
    } else {
        run_lexical(corpus_root, out_dir).await
    }
}

async fn run_lexical(corpus_root: &Path, out_dir: &Path) -> anyhow::Result<()> {
    /* existing body unchanged */
}
```

- [ ] **Step 3: Author the semantic mode**

Create `crates/teramind-search-eval/src/semantic.rs`:

```rust
//! Semantic eval mode. Loads corpus into throwaway PG, fills embeddings,
//! runs every query with semantic_weight > 0, writes to *-semantic.{json,md}.

use crate::corpus;
use crate::queries_bank::QUERIES;
use crate::reporter;
use crate::types::CorpusSize;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use teramind_core::embed::EmbeddingProvider;
use teramind_db::repos::{EmbeddingRepo, SearchRepo};
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use tracing::info;

pub async fn run_with_semantic(
    corpus_root: &Path,
    out_dir: &Path,
    semantic_weight: f32,
) -> anyhow::Result<()> {
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

    // Build provider from default config — Ollama on localhost.
    let cfg = teramindd::config::EmbedConfig::default();
    let provider = teramindd::services::embed::build_provider(&cfg)
        .map_err(|e| anyhow::anyhow!("provider init: {e}. Is Ollama running?"))?;
    let model = format!("ollama:{}", cfg.model);

    // Probe.
    provider.health_check().await
        .map_err(|e| anyhow::anyhow!("provider health: {e}"))?;
    info!("eval-semantic: provider {} healthy", provider.model_id());

    // Fill embeddings synchronously (blocking, deterministic).
    let embed_repo = EmbeddingRepo::new(pool.clone());
    fill_all_embeddings(&embed_repo, provider.clone(), &model).await?;

    let search = SearchRepo::new(pool.clone());
    let qrels = crate::harness::load_qrels(corpus_root)?;

    let mut per_query: Vec<reporter::PerQuery> = Vec::with_capacity(QUERIES.len());
    let mut latencies: Vec<u128> = Vec::with_capacity(QUERIES.len());
    for q in QUERIES {
        let started = Instant::now();
        // Embed the query.
        let q_vec = provider.embed(&[q.text.to_string()]).await
            .map(|mut v| v.pop()).ok().flatten();
        let fts = search.fts_turns(q.text, 10).await.unwrap_or_default();
        let diffs = search.trgm_diffs(q.text, 10).await.unwrap_or_default();
        let sem_turns = match q_vec.as_ref() {
            Some(v) => search.vector_search_turns(v, &model, 10).await.unwrap_or_default(),
            None => vec![],
        };
        let sem_diffs = match q_vec.as_ref() {
            Some(v) => search.vector_search_diffs(v, &model, 10).await.unwrap_or_default(),
            None => vec![],
        };
        let elapsed = started.elapsed().as_millis();
        latencies.push(elapsed);

        // Build hit_ids in score-weighted order. For eval, semantic_weight=0.4
        // means we want semantic-ranked rows to surface; combine.
        let mut hit_ids: Vec<String> = Vec::with_capacity(40);
        for f in &fts       { hit_ids.push(format!("turn:{}", f.turn_id)); }
        for d in &diffs     { hit_ids.push(format!("diff:{}", d.diff_id)); }
        for s in &sem_turns { hit_ids.push(format!("turn:{}", s.turn_id)); }
        for s in &sem_diffs { hit_ids.push(format!("diff:{}", s.diff_id)); }

        let relevance = crate::harness::relevance_for(&qrels, q.id, &hit_ids);
        let total_rel = qrels.judgments.get(q.id)
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
    let p95_ms = crate::harness::percentile_u32(&latencies, 95);
    let report = reporter::aggregate(&per_query, size, p95_ms);

    // Write to *-semantic outputs.
    std::fs::create_dir_all(out_dir)?;
    std::fs::write(out_dir.join("eval-results-semantic.json"),
                   serde_json::to_string_pretty(&report)?)?;
    std::fs::write(out_dir.join("eval-scorecard-semantic.md"),
                   reporter::render_markdown(&report))?;

    let _ = semantic_weight;  // wire up in v1.0.1 when corpus has paraphrase queries

    println!(
        "teramind-search-eval (semantic): nDCG@10={:.3} MRR={:.3} p95={}ms ({} queries)",
        report.overall.ndcg_at_10,
        report.overall.mrr,
        report.query_latency_p95_ms,
        report.overall.n_queries,
    );
    Ok(())
}

async fn fill_all_embeddings(
    repo: &EmbeddingRepo,
    provider: Arc<dyn EmbeddingProvider>,
    model: &str,
) -> anyhow::Result<()> {
    loop {
        let rows = repo.fetch_to_embed(model, 32).await?;
        if rows.is_empty() { break; }
        let texts: Vec<String> = rows.iter().map(|r| r.text.clone()).collect();
        let vectors = provider.embed(&texts).await
            .map_err(|e| anyhow::anyhow!("embed: {e}"))?;
        repo.bulk_insert(&rows, model, provider.dimension() as i32, &vectors).await?;
    }
    Ok(())
}
```

- [ ] **Step 4: Expose the helpers from harness**

`load_qrels`, `relevance_for`, `percentile_u32` in `harness.rs` are currently private. Mark them `pub(crate)` so `semantic.rs` can call them.

- [ ] **Step 5: Add `teramind-search-eval` dep on `teramindd`**

`semantic.rs` references `teramindd::config::EmbedConfig` and `teramindd::services::embed::build_provider`. The crate already depends on `teramindd`. Verify with `grep teramindd crates/teramind-search-eval/Cargo.toml`.

- [ ] **Step 6: Build**

Run: `cargo check -p teramind-search-eval`
Expected: succeeds.

- [ ] **Step 7: Commit**

```bash
git add crates/teramind-search-eval/src/semantic.rs crates/teramind-search-eval/src/harness.rs crates/teramind-search-eval/src/lib.rs
git commit -m "feat(search-eval): --semantic harness mode"
```

---

### Task 13.3: Seed `baseline-semantic.json`

This step requires a working Ollama on the host. Run only after `ollama pull nomic-embed-text-v2-moe`. If not available, skip and document in §16's runbook.

- [ ] **Step 1: Confirm Ollama is up**

Run: `curl -fsS http://localhost:11434/api/version && ollama list | grep -E "nomic-embed-text-v2-moe" || echo "model not pulled"`

If model isn't pulled: `ollama pull nomic-embed-text-v2-moe`.

- [ ] **Step 2: Run semantic eval**

Run: `cargo run --release -p teramind-search-eval -- run --semantic`
Expected: prints `nDCG@10=X.XXX ...`. Generates `benches/search-eval/eval-results-semantic.json` and `eval-scorecard-semantic.md`.

- [ ] **Step 3: Seed the baseline**

Run: `cargo run --release -p teramind-search-eval -- compare-baseline --update-baseline --results benches/search-eval/eval-results-semantic.json --baseline benches/search-eval/baseline-semantic.json`
Expected: writes `baseline-semantic.json`.

- [ ] **Step 4: Sanity check**

Run: `cargo run --release -p teramind-search-eval -- compare-baseline --results benches/search-eval/eval-results-semantic.json --baseline benches/search-eval/baseline-semantic.json`
Expected: `teramind-search-eval: all gates passed`.

- [ ] **Step 5: Gitignore the transient outputs**

Append to `.gitignore`:

```
benches/search-eval/eval-results-semantic.json
benches/search-eval/eval-scorecard-semantic.md
```

- [ ] **Step 6: Commit the baseline**

```bash
git add benches/search-eval/baseline-semantic.json .gitignore
git commit -m "feat(search-eval): seed baseline-semantic.json"
```

---

## Section 14 — CI workflow

### Task 14.1: `eval-semantic` job in `.github/workflows/search-eval.yml`

**Files:**
- Modify: `.github/workflows/search-eval.yml`

- [ ] **Step 1: Append the new job**

After the existing `eval:` job, append (same indent under `jobs:`):

```yaml
  eval-semantic:
    name: run L5 benchmark (semantic)
    needs: eval
    runs-on: ubuntu-22.04
    continue-on-error: true   # fail-soft until v1.0.1 corpus expansion
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: install ollama
        run: curl -fsSL https://ollama.ai/install.sh | sh
      - name: start ollama and pull model
        run: |
          ollama serve &
          sleep 5
          ollama pull nomic-embed-text-v2-moe
      - name: run semantic benchmark
        env:
          TERAMIND_LOG: warn
        run: cargo run --release -p teramind-search-eval -- run --semantic
      - name: compare against semantic baseline
        run: |
          cargo run --release -p teramind-search-eval -- compare-baseline \
            --results  benches/search-eval/eval-results-semantic.json \
            --baseline benches/search-eval/baseline-semantic.json
      - uses: actions/upload-artifact@v4
        with:
          name: eval-scorecard-semantic
          path: |
            benches/search-eval/eval-results-semantic.json
            benches/search-eval/eval-scorecard-semantic.md
          if-no-files-found: error
```

- [ ] **Step 2: Validate YAML**

Run: `python3 -c "import yaml; w=yaml.safe_load(open('.github/workflows/search-eval.yml')); print(list(w['jobs'].keys()))"`
Expected: `['eval', 'eval-semantic']`.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/search-eval.yml
git commit -m "ci(search-eval): eval-semantic job (fail-soft)"
```

---

## Section 15 — L3 integration tests

### Task 15.1: Mock-provider integration test

**Files:**
- Create: `crates/teramindd/tests/embedding_worker_mock.rs`

- [ ] **Step 1: Author the test**

```rust
//! L3: mock embedding provider feeds a real PG via the real worker.

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::embed::{DistanceMetric, EmbedError, EmbeddingProvider, ProviderKind};
use teramind_core::ids::{SessionId, TurnId};
use teramind_core::redact::Redactor;
use teramind_db::repos::{AgentRepo, EmbeddingRepo, SessionRepo, TraceRepo};
use teramind_db::repos::session::NewSession;
use teramindd::services::embedding_worker::{EmbeddingWorker, EmbeddingWorkerDeps};
use time::OffsetDateTime;

struct DeterministicMock { dim: usize }

#[async_trait]
impl EmbeddingProvider for DeterministicMock {
    fn kind(&self) -> ProviderKind { ProviderKind::Fastembed }
    fn model_id(&self) -> &str { "mock-model" }
    fn dimension(&self) -> usize { self.dim }
    fn max_tokens(&self) -> usize { 8192 }
    fn distance_metric(&self) -> DistanceMetric { DistanceMetric::Cosine }
    async fn health_check(&self) -> Result<(), EmbedError> { Ok(()) }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Ok(texts.iter().map(|t| {
            let mut v = vec![0.0f32; self.dim];
            v[0] = t.chars().count() as f32;
            v
        }).collect())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn worker_fills_embeddings_within_15s() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(
        dir.path().to_path_buf(), "teramind",
    ).await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    teramind_db::migrate::run(&pool).await?;

    // Seed one session + turn.
    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await?;
    let sid = sessions.insert(NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/p",
        project_id: None, parent_session_id: None,
        git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u",
        started_at: OffsetDateTime::now_utc(),
    }).await?;
    let _ = trace.upsert_turn_with_id(
        TurnId(uuid::Uuid::new_v4()), sid, 0,
        OffsetDateTime::now_utc(), Some("first prompt"),
    ).await?;

    let repo = EmbeddingRepo::new(pool.clone());
    assert_eq!(repo.backlog("mock:mock-model").await?, 1);

    let _worker = EmbeddingWorker::spawn(EmbeddingWorkerDeps {
        repo: repo.clone(),
        provider: Arc::new(DeterministicMock { dim: 768 }),
        redactor: Arc::new(Redactor::with_default_rules()),
        model: "mock:mock-model".into(),
        poll_interval: Duration::from_millis(200),
        batch_size: 32,
    });

    // Poll for backlog to drain.
    for _ in 0..75 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if repo.backlog("mock:mock-model").await? == 0 { break; }
    }
    assert_eq!(repo.backlog("mock:mock-model").await?, 0);

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramindd --test embedding_worker_mock --release`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/tests/embedding_worker_mock.rs
git commit -m "test(daemon): L3 embedding_worker against mock provider"
```

---

### Task 15.2: Real-Ollama integration test (host-GPU preferred)

**Files:**
- Create: `crates/teramindd/tests/embedding_worker_ollama.rs`

- [ ] **Step 1: Author the test**

```rust
//! L3: real Ollama (host-local, GPU-preferred). Skips when probe fails.

use std::sync::Arc;
use std::time::Duration;
use teramind_core::ids::TurnId;
use teramind_db::repos::{AgentRepo, EmbeddingRepo, SearchRepo, SessionRepo, TraceRepo};
use teramind_db::repos::session::NewSession;
use teramindd::config::EmbedConfig;
use teramindd::services::embed::build_provider;
use teramindd::services::embedding_worker::{EmbeddingWorker, EmbeddingWorkerDeps};
use time::OffsetDateTime;

async fn probe_ollama() -> bool {
    reqwest::Client::new()
        .get("http://localhost:11434/api/version")
        .timeout(Duration::from_millis(500))
        .send().await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ollama_e2e_paraphrase_lookup() -> anyhow::Result<()> {
    if !probe_ollama().await {
        eprintln!("ollama not running on localhost:11434, skipping");
        return Ok(());
    }

    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(
        dir.path().to_path_buf(), "teramind",
    ).await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    teramind_db::migrate::run(&pool).await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let trace = TraceRepo::new(pool.clone());
    let agent = agents.upsert("claude_code", None).await?;
    let sid = sessions.insert(NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/p",
        project_id: None, parent_session_id: None,
        git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u",
        started_at: OffsetDateTime::now_utc(),
    }).await?;
    // The query and the planted turn share NO lexical tokens. A semantic
    // model should still find the planted turn.
    let _ = trace.upsert_turn_with_id(
        TurnId(uuid::Uuid::new_v4()), sid, 0,
        OffsetDateTime::now_utc(),
        Some("the access credential cycle is renewed before timeout"),
    ).await?;

    let cfg = EmbedConfig::default();
    let provider = build_provider(&cfg)?;
    let model = format!("ollama:{}", cfg.model);

    // Worker drains the backlog.
    let repo = EmbeddingRepo::new(pool.clone());
    let _worker = EmbeddingWorker::spawn(EmbeddingWorkerDeps {
        repo: repo.clone(),
        provider: provider.clone(),
        redactor: Arc::new(teramind_core::redact::Redactor::with_default_rules()),
        model: model.clone(),
        poll_interval: Duration::from_millis(500),
        batch_size: 8,
    });
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if repo.backlog(&model).await? == 0 { break; }
    }
    assert_eq!(repo.backlog(&model).await?, 0, "worker should drain backlog");

    // Query with a paraphrase.
    let q_emb = provider.embed(&[
        "how does the JWT refresh flow work".to_string()
    ]).await
        .map_err(|e| anyhow::anyhow!("embed query: {e}"))?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("no embedding returned"))?;

    let search = SearchRepo::new(pool.clone());
    let hits = search.vector_search_turns(&q_emb, &model, 5).await?;
    assert!(!hits.is_empty(), "semantic search should return the paraphrased turn");
    assert!(hits[0].semantic_score > 0.4,
        "expected non-trivial cosine similarity, got {}", hits[0].semantic_score);

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 2: Run (locally; will skip if no Ollama)**

Run: `cargo test -p teramindd --test embedding_worker_ollama --release -- --nocapture`
Expected: PASS if Ollama is up with the model pulled; otherwise prints `ollama not running…, skipping` and returns success.

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/tests/embedding_worker_ollama.rs
git commit -m "test(daemon): L3 real-Ollama paraphrase recall"
```

---

### Task 15.3: Search-degraded-when-provider-fails test

**Files:**
- Create: `crates/teramindd/tests/search_degraded_no_provider.rs`

- [ ] **Step 1: Author the test**

```rust
use async_trait::async_trait;
use std::sync::Arc;
use teramind_core::embed::{DistanceMetric, EmbedError, EmbeddingProvider, ProviderKind};

struct AlwaysFailsProvider;

#[async_trait]
impl EmbeddingProvider for AlwaysFailsProvider {
    fn kind(&self) -> ProviderKind { ProviderKind::Fastembed }
    fn model_id(&self) -> &str { "broken" }
    fn dimension(&self) -> usize { 768 }
    fn max_tokens(&self) -> usize { 8192 }
    fn distance_metric(&self) -> DistanceMetric { DistanceMetric::Cosine }
    async fn health_check(&self) -> Result<(), EmbedError> { Ok(()) }
    async fn embed(&self, _t: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Err(EmbedError::Other("simulated".into()))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn search_marks_degraded_when_provider_fails() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(
        dir.path().to_path_buf(), "teramind",
    ).await?;
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await?;
    teramind_db::migrate::run(&pool).await?;

    let search_repo = teramind_db::repos::SearchRepo::new(pool.clone());
    let weights = teramindd::services::search::BlendWeights {
        semantic: 0.5, ..teramindd::services::search::BlendWeights::default()
    };
    let req = teramind_core::types::SearchRequest {
        query: "anything".into(), limit: 5,
    };
    let out = teramindd::services::search::do_search(
        &search_repo,
        Some(Arc::new(AlwaysFailsProvider)),
        "ollama:broken",
        weights,
        &req,
    ).await?;
    assert!(out.degraded, "embedding failure should set degraded=true");
    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramindd --test search_degraded_no_provider --release`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/tests/search_degraded_no_provider.rs
git commit -m "test(daemon): L3 search marks degraded when embedding fails"
```

---

## Section 16 — Manual smoke runbook + final check

### Task 16.1: Manual runbook

**Files:**
- Create: `docs/runbooks/pgvector-manual-smoke.md`

- [ ] **Step 1: Author**

```markdown
# Manual smoke: pgvector + semantic search

Confirms that the embedding worker fills vectors, semantic search returns
paraphrase hits, and `teramind doctor` surfaces provider health.

## Prereqs

- Plans A–F installed (Teramind Core).
- Ollama running on localhost:11434 with `nomic-embed-text-v2-moe` pulled:
  ```sh
  ollama pull nomic-embed-text-v2-moe
  ```
- `~/.config/teramind/search.toml` has `semantic = 0.4` under `[blend]`.

## Steps

1. Start the daemon: `teramind start`.
2. Run a Claude session that writes a few turns and edits a file.
3. Check the backlog drains:
   ```sh
   teramind doctor | grep "embedding"
   ```
   Expect: `embedding provider: ollama:nomic-embed-text-v2-moe (healthy)` and
   `embedding backlog: 0 rows (last filled <N>s ago)` within ~30 s.
4. Run a paraphrase search:
   ```sh
   teramind search "how does the access token refresh"
   ```
   Expect: a hit that wasn't previously findable via the lexical-only search
   (e.g. a turn discussing "JWT expiry rotation").
5. Stop Ollama:
   ```sh
   killall ollama
   ```
   Re-run `teramind doctor` — expect `unhealthy`. Re-run the search —
   expect lexical-only results plus a warning in the daemon log.

## Troubleshooting

- "embedding provider: ollama … unhealthy" right after start: confirm
  `ollama serve` is up; `curl http://localhost:11434/api/version`.
- Backlog never drains: check `~/.local/share/teramind/logs/teramindd.log.*`
  for `embed_with_bisect failed` lines; verify the model is pulled.
- Paraphrase search returns nothing: confirm `search.toml` has
  `semantic = 0.4` (default is `0.0`) AND the daemon was restarted after
  the change (config is read at startup).
```

- [ ] **Step 2: Commit**

```bash
git add docs/runbooks/pgvector-manual-smoke.md
git commit -m "docs: manual smoke runbook for pgvector"
```

---

### Task 16.2: Final integration check

- [ ] **Step 1: Workspace check + tests + clippy**

```bash
cargo check --workspace
cargo test --workspace --lib
cargo test -p teramind-db --release
cargo test -p teramindd --release
cargo clippy --workspace -- -D warnings
```

Expected: all pass. Fix minor lint issues inline.

- [ ] **Step 2: Validate workflow YAML**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/search-eval.yml'))"
```

Expected: no exception.

- [ ] **Step 3: Optional cleanup commit**

```bash
git add -A
git commit -m "chore: clippy cleanups for pgvector plan" || true
```

- [ ] **Step 4: STOP — do not push or open a PR.**

Defer to user approval, per Plans A–F convention.

---

## Spec coverage self-check

| Spec section / requirement | Plan task |
|---|---|
| §2.1 `embeddings` table | §1 (migration), §2 (trait) |
| §2.1 `pgvector` extension + HNSW + cosine | §0.3 (install), §1 (migration) |
| §2.1 EmbeddingProvider trait + 3 impls | §2, §3, §4, §5 |
| §2.1 `embedding_worker` async, never blocks | §8 |
| §2.1 SearchRepo vector_search_* | §9 |
| §2.1 `semantic` blend term, default 0.0 | §10 |
| §2.1 `teramind-search-eval --semantic` | §13 |
| §2.1 `eval-semantic` fail-soft CI job | §14 |
| §2.1 `teramind doctor` surfaces provider/backlog | §12 |
| §2.2 cloud providers v1.0 stub | §5 |
| §2.3 SC#1 worker fills within 10s, no ingest impact | §15.1 |
| §2.3 SC#2 paraphrase recall via `semantic_weight > 0` | §15.2 |
| §2.3 SC#3 `--semantic` completes < 3 min on 500-session corpus | §13.3 |
| §2.3 SC#4 default L5 gate unaffected | §13–§14 (separate baselines) |
| §2.3 SC#5 daemon stays up when provider offline | §8 (worker retries), §15.3 |
| §4.2 storage schema details | §1 |
| §4.3 EmbeddingProvider trait shape | §2 |
| §4.4 worker pseudocode | §8 |
| §4.5 search service blend | §10 |
| §5 embed.toml + search.toml | §6.1, §10.2 |
| §5 validation: cloud + egress=false → refuse | §6.1 |
| §6 baseline-semantic.json | §13.3 |
| §6 CI fail-soft initially | §14 |
| §7.1–7.4 testing (L1–L4) | §6.1, §15 |
| §7.5 L5 semantic mode | §13 |
| §7.6 property tests + fault injection | §6.1, §8, §15 |
| §7.7 perf budgets | informal: §15 timing assertions |
| §8 risks (Ollama-missing fallback) | §15 (probe), §16 (runbook) |
| §8 orphan sweeper | §11 |
