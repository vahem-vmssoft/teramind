use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "teramind-sync-server", version)]
struct Cli {
    /// Path to config TOML (defaults to /etc/teramind-sync-server/config.toml).
    #[arg(long, env = "TERAMIND_SYNC_CONFIG")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run database migrations against the configured Postgres.
    Migrate,
    /// Start the HTTP(S) server.
    Serve {
        /// Bind only HTTP (no TLS). Insecure; loud flag for dev only.
        #[arg(long)]
        insecure_allow_http: bool,
    },
    /// Print version.
    Version,
}

fn init_logging() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("TERAMIND_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .json().init();
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_logging();
    match cli.cmd {
        Cmd::Version => {
            println!("teramind-sync-server {}", teramind_sync_server::VERSION);
            Ok(())
        }
        Cmd::Migrate => {
            let cfg_path = cli.config.unwrap_or_else(default_config_path);
            let cfg = teramind_sync_server::config::ServerConfig::load(&cfg_path)?;
            let pool = teramind_db::pool::DbPool::connect_url(&cfg.database_url).await?;
            teramind_db::migrate::run(&pool).await?;
            println!("migrations OK");
            Ok(())
        }
        Cmd::Serve { insecure_allow_http } => {
            let cfg_path = cli.config.unwrap_or_else(default_config_path);
            let cfg = teramind_sync_server::config::ServerConfig::load(&cfg_path)?;
            if cfg.tls.is_none() && !insecure_allow_http {
                anyhow::bail!("TLS not configured; pass --insecure-allow-http to opt into plaintext HTTP (dev only)");
            }
            let pool = teramind_db::pool::DbPool::connect_url(&cfg.database_url).await?;
            teramind_db::migrate::run(&pool).await?;
            let addr: SocketAddr = cfg.listen_addr.parse()?;
            let state = teramind_sync_server::state::AppState::new(pool, cfg.clone());
            if let Some(tls) = cfg.tls.as_ref() {
                teramind_sync_server::server::serve_tls(state, addr, tls).await
            } else {
                teramind_sync_server::server::serve(state, addr).await
            }
        }
    }
}

fn default_config_path() -> PathBuf {
    PathBuf::from("/etc/teramind-sync-server/config.toml")
}
