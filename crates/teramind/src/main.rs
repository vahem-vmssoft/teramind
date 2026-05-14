mod cli;
mod commands;
mod ipc;

use clap::Parser;
use cli::{Cli, Command};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::try_init().ok();
    let cli = Cli::parse();
    match cli.command {
        Command::Init => commands::init::run().await,
        Command::Start => commands::start::run().await,
        Command::Stop => commands::stop::run().await,
        Command::Status { format } => commands::status::run(format).await,
        Command::Version => commands::version::run().await,
        Command::Restart => commands::restart::run().await,
        Command::Doctor => commands::doctor::run().await,
        Command::Reset { purge, confirm } => commands::reset::run(purge, confirm).await,
        Command::Claude { action } => match action {
            cli::ClaudeAction::Install   => commands::claude::install().await,
            cli::ClaudeAction::Uninstall => commands::claude::uninstall().await,
        },
        Command::Search { query, limit, json, grep } =>
            commands::search::run(query, limit, json, grep).await,
        Command::Uninstall { purge, confirm } => commands::uninstall::run(purge, confirm).await,
    }
}
