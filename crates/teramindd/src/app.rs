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
            time::Duration::seconds(5),
        );
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

        let handler = Arc::new(DaemonIpcHandler {
            ingest: ingest.clone(),
            stats: stats.clone(),
            started: Instant::now(),
            last_pg_bytes: 0.into(),
            last_jsonl_bytes: 0.into(),
            search_repo: teramind_db::repos::SearchRepo::new(pool.clone()),
            jsonl_dir: paths.raw_dir.clone(),
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
