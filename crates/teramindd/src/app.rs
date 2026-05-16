use crate::config::Config;
use crate::paths::Paths;
use crate::services::ingest::{IngestDeps, IngestService, IngestStats};
use crate::services::ipc_server::{run_accept_loop, DaemonIpcHandler};
use crate::services::jsonl_writer::JsonlWriter;
use crate::services::session_manager::SessionManager;
use crate::services::storage_stats;
use anyhow::Context;
use std::sync::Arc;
use std::time::{Duration, Instant};
use teramind_core::redact::Redactor;
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, StorageStatsRepo, TraceRepo};
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramind_ipc::transport::listen;
use tracing::info;

pub struct App;

impl App {
    pub async fn run() -> anyhow::Result<()> {
        let paths = Paths::resolve()?;
        paths.ensure_dirs()?;
        let config_path = paths.config_dir.join("config.toml");
        let config = Config::load_or_default(&config_path)?;

        let file_appender = tracing_appender::rolling::daily(&paths.logs_dir, "teramindd.log");
        let (nb, guard) = tracing_appender::non_blocking(file_appender);
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_env("TERAMIND_LOG")
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
            )
            .with_writer(nb)
            .json()
            .init();
        std::mem::forget(guard);

        info!("teramindd starting");

        let pid = std::process::id();
        std::fs::write(&paths.pid_file, format!("{pid}\n")).context("write pid file")?;

        let supervisor = PgSupervisor::start(paths.pgdata_dir.clone(), "teramind").await?;
        let pool = DbPool::connect(supervisor.connect_options()).await?;
        migrate::run(&pool).await?;

        let jsonl = Arc::new(JsonlWriter::open(paths.raw_dir.clone()).await?);
        let stats = Arc::new(IngestStats::default());
        let write_tool_ring = crate::services::write_tool_ring::WriteToolRing::new(
            64,
            time::Duration::milliseconds(config.fs_attribution_window_ms as i64),
        );

        // FS watcher pipeline: raw -> debounced -> resolved -> handle_event
        let (raw_tx, mut raw_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::services::fs_watcher::RawEvent>();
        let (resolved_tx, resolved_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::services::fs_watcher::RawEvent>();
        let debouncer = std::sync::Arc::new(
            crate::services::fs_watcher::Debouncer::start(
                std::time::Duration::from_millis(config.fs_debounce_ms),
                resolved_tx,
            ),
        );
        let gaps_counter: std::sync::Arc<std::sync::atomic::AtomicU64> =
            std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let registry = std::sync::Arc::new(
            crate::services::fs_watcher::WatchRegistry::new(raw_tx, gaps_counter.clone()),
        );

        {
            let s = stats.clone();
            let g = gaps_counter.clone();
            tokio::spawn(async move {
                loop {
                    let v = g.load(std::sync::atomic::Ordering::Relaxed);
                    s.fs_watcher_gaps.store(v, std::sync::atomic::Ordering::Relaxed);
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            });
        }

        let ingest = Arc::new(IngestService::spawn(
            config.ingest_queue_capacity,
            IngestDeps {
                redactor: Arc::new(Redactor::with_default_rules()),
                jsonl: jsonl.clone(),
                sessions: SessionManager::new(),
                agents: AgentRepo::new(pool.clone()),
                session_repo: SessionRepo::new(pool.clone()),
                trace: TraceRepo::new(pool.clone()),
                diffs: DiffRepo::new(pool.clone()),
                stats: stats.clone(),
                dead_letter_dir: paths.dead_letter_dir.clone(),
                write_tool_ring: write_tool_ring.clone(),
                fs_registry: registry.clone(),
            },
        ));
        let drained = crate::services::ingest::drain_inbox(&paths.inbox_dir, &ingest)
            .await
            .unwrap_or(0);
        if drained > 0 {
            tracing::info!(drained, "drained inbox events");
        }
        storage_stats::spawn(
            StorageStatsRepo::new(pool.clone()),
            paths.raw_dir.clone(),
            "teramind".into(),
            Duration::from_secs(config.storage_sample_interval_secs),
        );

        // Embedding worker.
        let embed_cfg_path = paths.config_dir.join("embed.toml");
        let embed_cfg = crate::config::EmbedConfig::load_or_default(&embed_cfg_path)?;
        let provider = crate::services::embed::build_provider(&embed_cfg)?;
        let embed_model_db_key = format!("{}:{}", provider_prefix(provider.kind()), embed_cfg.model);
        let embed_repo = teramind_db::repos::EmbeddingRepo::new(pool.clone());
        let embed_worker = crate::services::embedding_worker::EmbeddingWorker::spawn(
            crate::services::embedding_worker::EmbeddingWorkerDeps {
                repo: embed_repo.clone(),
                provider: provider.clone(),
                redactor: std::sync::Arc::new(teramind_core::redact::Redactor::with_default_rules()),
                model: embed_model_db_key.clone(),
                poll_interval: std::time::Duration::from_secs(embed_cfg.poll_interval_secs),
                batch_size: embed_cfg.batch_size,
            },
        );
        let embed_stats = embed_worker.stats.clone();
        let _embed_worker_guard = embed_worker;

        // Session summarizer.
        let summarize_cfg_path = paths.config_dir.join("summarize.toml");
        let summarize_cfg = crate::config::SummarizeConfig::load_or_default(&summarize_cfg_path)?;
        let secrets_path = paths.config_dir.join("secrets.toml");
        let summary_provider = crate::services::summarize::build_provider(
            &summarize_cfg, &secrets_path,
        )?;
        let summarize_model_db_key = format!(
            "{}:{}",
            provider_prefix(summary_provider.kind()),
            summarize_cfg.model,
        );
        let wiki_repo = teramind_db::repos::WikiRepo::new(pool.clone());
        let summarizer = crate::services::summarizer_worker::SummarizerWorker::spawn(
            crate::services::summarizer_worker::SummarizerDeps {
                repo: wiki_repo.clone(),
                provider: summary_provider.clone(),
                redactor: std::sync::Arc::new(teramind_core::redact::Redactor::with_default_rules()),
                model: summarize_model_db_key.clone(),
                poll_interval: std::time::Duration::from_secs(summarize_cfg.poll_interval_secs),
                min_turns: summarize_cfg.min_turns,
                min_duration_secs: summarize_cfg.min_duration_secs,
                input_char_budget: summarize_cfg.input_char_budget,
                output_token_budget: summarize_cfg.output_token_budget,
            },
        );
        let summarizer_stats = summarizer.stats.clone();
        let _summarizer_guard = summarizer;  // hold for App::run lifetime

        let orphan_interval = std::time::Duration::from_secs(
            embed_cfg.orphan_sweep_interval_hr as u64 * 3600,
        );
        let _orphan_guard = crate::services::orphan_sweeper::OrphanSweeper::spawn(
            embed_repo.clone(),
            orphan_interval,
        );

        // Pump raw -> debouncer.
        {
            let deb = debouncer.clone();
            tokio::spawn(async move {
                while let Some(ev) = raw_rx.recv().await {
                    deb.enqueue(ev).await;
                }
            });
        }

        let snapshot_cache = crate::services::snapshot_cache::SnapshotCache::new(
            time::Duration::seconds(config.fs_snapshot_ttl_secs as i64),
        );

        crate::services::fs_watcher::FsWatcherService::spawn(
            crate::services::fs_watcher::FsWatcherDeps {
                registry: registry.clone(),
                debouncer: debouncer.clone(),
                cache: snapshot_cache.clone(),
                ring: write_tool_ring.clone(),
                ingest_tx: ingest.clone(),
            },
            resolved_rx,
        );

        let search_cfg_path = paths.config_dir.join("search.toml");
        let search_weights = crate::config::load_search_weights(&search_cfg_path)?;

        let handler = Arc::new(DaemonIpcHandler {
            ingest: ingest.clone(),
            stats: stats.clone(),
            started: Instant::now(),
            last_pg_bytes: 0.into(),
            last_jsonl_bytes: 0.into(),
            search_repo: teramind_db::repos::SearchRepo::new(pool.clone()),
            jsonl_dir: paths.raw_dir.clone(),
            embed_provider: provider.clone(),
            embed_model: embed_model_db_key.clone(),
            search_weights,
            embed_stats,
        });
        let listener = listen(&paths.socket_path)?;
        let h2 = handler.clone();
        tokio::spawn(async move {
            let _ = run_accept_loop(listener, h2).await;
        });

        crate::signals::shutdown_signal().await;
        info!("teramindd shutting down");
        let _ = std::fs::remove_file(&paths.pid_file);
        let _ = std::fs::remove_file(&paths.socket_path);
        supervisor.shutdown().await?;
        Ok(())
    }
}

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
