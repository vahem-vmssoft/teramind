use teramind::cli::{ClaudeAction, Cli, Command, SessionsAction};
use teramind::commands;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::try_init().ok();
    let cli = Cli::parse();
    match cli.command {
        Command::Init {
            team,
            server,
            invite,
            device_name,
        } => commands::init::run(team, server, invite, device_name).await,
        Command::Start => commands::start::run().await,
        Command::Stop => commands::stop::run().await,
        Command::Status { format } => commands::status::run(format).await,
        Command::Version => commands::version::run().await,
        Command::Restart => commands::restart::run().await,
        Command::Doctor => commands::doctor::run().await,
        Command::Reset { purge, confirm } => commands::reset::run(purge, confirm).await,
        Command::Claude { action } => match action {
            ClaudeAction::Install => commands::claude::install().await,
            ClaudeAction::Uninstall => commands::claude::uninstall().await,
        },
        Command::Search {
            query,
            limit,
            json,
            grep,
        } => commands::search::run(query, limit, json, grep).await,
        Command::Uninstall { purge, confirm } => commands::uninstall::run(purge, confirm).await,
        Command::SelfUpdate { check_only, force } => {
            commands::self_update::run(check_only, force).await
        }
        Command::Sessions { action } => match action {
            SessionsAction::Show { session_id, json } => {
                commands::sessions::show(session_id, json).await
            }
        },
    }
}
