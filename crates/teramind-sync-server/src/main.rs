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
    /// Manage invite codes.
    Invite {
        #[command(subcommand)]
        action: InviteAction,
    },
    /// Manage members + devices.
    Member {
        #[command(subcommand)]
        action: MemberAction,
    },
}

#[derive(Subcommand)]
enum InviteAction {
    /// Create a new invite for an email.
    Create {
        #[arg(long)]
        email: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        created_by: Option<String>,
        #[arg(long)]
        expires_in_days: Option<i64>,
    },
    /// List outstanding invites.
    List,
    /// Revoke an invite by id.
    Revoke { id: String },
}

#[derive(Subcommand)]
enum MemberAction {
    /// List users + device counts.
    List,
    /// Revoke a single device by id.
    RevokeDevice { id: String },
    /// Revoke a user (cascade-revokes auth lookups for their devices).
    RevokeUser { id: String },
}

fn init_logging() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("TERAMIND_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .json()
        .init();
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
        Cmd::Serve {
            insecure_allow_http,
        } => {
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
        Cmd::Invite { action } => {
            let cfg = teramind_sync_server::config::ServerConfig::load(
                &cli.config.unwrap_or_else(default_config_path),
            )?;
            let ctx = teramind_sync_server::admin::AdminCtx::open(cfg).await?;
            match action {
                InviteAction::Create {
                    email,
                    name,
                    created_by,
                    expires_in_days,
                } => {
                    teramind_sync_server::admin::invite_create(
                        &ctx,
                        &email,
                        name.as_deref(),
                        created_by.as_deref(),
                        expires_in_days,
                    )
                    .await
                }
                InviteAction::List => teramind_sync_server::admin::invite_list(&ctx).await,
                InviteAction::Revoke { id } => {
                    teramind_sync_server::admin::invite_revoke(&ctx, &id).await
                }
            }
        }
        Cmd::Member { action } => {
            let cfg = teramind_sync_server::config::ServerConfig::load(
                &cli.config.unwrap_or_else(default_config_path),
            )?;
            let ctx = teramind_sync_server::admin::AdminCtx::open(cfg).await?;
            match action {
                MemberAction::List => teramind_sync_server::admin::member_list(&ctx).await,
                MemberAction::RevokeDevice { id } => {
                    teramind_sync_server::admin::member_revoke_device(&ctx, &id).await
                }
                MemberAction::RevokeUser { id } => {
                    teramind_sync_server::admin::member_revoke_user(&ctx, &id).await
                }
            }
        }
    }
}

fn default_config_path() -> PathBuf {
    PathBuf::from("/etc/teramind-sync-server/config.toml")
}
