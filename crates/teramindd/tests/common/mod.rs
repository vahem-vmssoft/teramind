//! Shared test scaffolding for L3 integration tests.
//! Spins up a real embedded Postgres + the daemon services in-process
//! and returns handles for driving events and asserting state.

use std::path::PathBuf;
use std::sync::Arc;
use teramind_core::redact::Redactor;
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo};
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramindd::services::fs_watcher::{Debouncer, FsWatcherDeps, FsWatcherService, WatchRegistry};
use teramindd::services::ingest::{IngestDeps, IngestService, IngestStats};
use teramindd::services::jsonl_writer::JsonlWriter;
use teramindd::services::session_manager::SessionManager;
use teramindd::services::snapshot_cache::SnapshotCache;
use teramindd::services::write_tool_ring::WriteToolRing;

pub struct Harness {
    pub pool: DbPool,
    pub ingest: Arc<IngestService>,
    // Kept to hold the registry alive; tests drive it via ingest.
    #[allow(dead_code)]
    pub registry: Arc<WatchRegistry>,
    pub _sup: PgSupervisor,
    pub _tmp: tempfile::TempDir,
    // Kept for potential diagnostics / future tests; not read yet.
    #[allow(dead_code)]
    pub raw_dir: PathBuf,
    #[allow(dead_code)]
    pub dead_letter_dir: PathBuf,
}

impl Harness {
    pub async fn start() -> anyhow::Result<Self> {
        let tmp = tempfile::tempdir()?;
        // Canonicalize the tmp path so macOS symlink /var -> /private/var is resolved.
        // Without this, notify reports events under the canonical path but the registry
        // stores the non-canonical path, causing strip_prefix to fail silently.
        let tmp_canon = tmp.path().canonicalize()?;

        let raw_dir = tmp_canon.join("raw");
        std::fs::create_dir_all(&raw_dir)?;
        let dead_letter_dir = tmp_canon.join("dl");
        std::fs::create_dir_all(&dead_letter_dir)?;
        let pgdata = tmp_canon.join("pgdata");

        let sup = PgSupervisor::start(pgdata, "teramind").await?;
        let pool = DbPool::connect(sup.connect_options()).await?;
        migrate::run(&pool).await?;

        let stats = Arc::new(IngestStats::default());
        let jsonl = Arc::new(JsonlWriter::open(raw_dir.clone()).await?);
        let write_tool_ring = WriteToolRing::new(64, time::Duration::seconds(5));

        let (raw_tx, mut raw_rx) = tokio::sync::mpsc::unbounded_channel();
        let (resolved_tx, resolved_rx) = tokio::sync::mpsc::unbounded_channel();
        let debouncer = Arc::new(Debouncer::start(
            std::time::Duration::from_millis(100),
            resolved_tx,
        ));
        let gaps_counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let registry = Arc::new(WatchRegistry::new(raw_tx, gaps_counter));

        let deb = debouncer.clone();
        tokio::spawn(async move {
            while let Some(ev) = raw_rx.recv().await {
                deb.enqueue(ev).await;
            }
        });

        let snapshot_cache = SnapshotCache::new(time::Duration::seconds(60));

        let ingest = Arc::new(IngestService::spawn(
            1024,
            IngestDeps {
                redactor: Arc::new(Redactor::with_default_rules()),
                jsonl: jsonl.clone(),
                sessions: SessionManager::new(),
                agents: AgentRepo::new(pool.clone()),
                session_repo: SessionRepo::new(pool.clone()),
                trace: TraceRepo::new(pool.clone()),
                diffs: DiffRepo::new(pool.clone()),
                stats: stats.clone(),
                dead_letter_dir: dead_letter_dir.clone(),
                write_tool_ring: write_tool_ring.clone(),
                fs_registry: registry.clone(),
            },
        ));

        FsWatcherService::spawn(
            FsWatcherDeps {
                registry: registry.clone(),
                debouncer: debouncer.clone(),
                cache: snapshot_cache.clone(),
                ring: write_tool_ring.clone(),
                ingest_tx: ingest.clone(),
            },
            resolved_rx,
        );

        Ok(Harness {
            pool,
            ingest,
            registry,
            _sup: sup,
            _tmp: tmp,
            raw_dir,
            dead_letter_dir,
        })
    }
}
