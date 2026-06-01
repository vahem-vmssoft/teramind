use teramind::cli::{
    ClaudeAction, Cli, Command, RedactAction, SessionsAction, SkillsAction, TeamAction,
};
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
        Command::Feed { follow, backlog } => commands::feed::run(follow, backlog).await,
        Command::Skills { action } => match action {
            SkillsAction::List { filter, limit } => commands::skills::list(filter, limit).await,
            SkillsAction::Show { name_or_id } => commands::skills::show(name_or_id).await,
            SkillsAction::Observations {
                kind,
                min_freq,
                status,
                limit,
            } => commands::skills::observations(kind, min_freq, status, limit).await,
        },
        Command::Redact { action } => match action {
            RedactAction::Test { input } => commands::redact::test(input).await,
        },
        Command::Team { action } => match action {
            TeamAction::ShareSet { enable, disable } => {
                commands::team::share_set(enable, disable).await
            }
        },
    }
}
