use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "teramind", version, about = "Teramind CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Initialize Teramind data + config directories and run migrations.
    Init {
        /// Opt into team mode: redeem an invite + generate a device key.
        #[arg(long)]
        team: bool,
        /// Sync server URL (required with --team).
        #[arg(long, requires = "team")]
        server: Option<String>,
        /// Invite code from the team admin (required with --team).
        #[arg(long, requires = "team")]
        invite: Option<String>,
        /// Optional device name (defaults to hostname).
        #[arg(long, requires = "team")]
        device_name: Option<String>,
    },
    /// Start the daemon in the background (lazy-spawn).
    Start,
    /// Stop the running daemon.
    Stop,
    /// Show daemon status.
    Status {
        #[arg(long)]
        format: Option<String>,
    },
    /// Print version.
    Version,
    /// Restart (stop + start).
    Restart,
    /// Run diagnostic checks and print a pasteable report.
    Doctor,
    /// Reset local data. With --purge, also remove plugin and config.
    Reset {
        #[arg(long)]
        purge: bool,
        #[arg(long)]
        confirm: bool,
    },
    /// Manage the Claude Code plugin integration.
    Claude {
        #[command(subcommand)]
        action: ClaudeAction,
    },
    /// Search prior traces and skills.
    Search {
        /// The query text.
        query: String,
        /// Maximum hits to return.
        #[arg(short, long, default_value = "10")]
        limit: u32,
        /// Output as JSON instead of pretty text.
        #[arg(long)]
        json: bool,
        /// Force the grep fallback path.
        #[arg(long)]
        grep: bool,
    },
    /// Inspect ended sessions.
    Sessions {
        #[command(subcommand)]
        action: SessionsAction,
    },
    /// Uninstall the Teramind binaries. With --purge, also remove data + config.
    Uninstall {
        /// Also remove `~/.local/share/teramind/` and `~/.config/teramind/`.
        #[arg(long)]
        purge: bool,
        /// Skip the interactive confirmation.
        #[arg(long)]
        confirm: bool,
    },
    /// Check for and apply Teramind updates.
    SelfUpdate {
        /// Don't actually replace anything; just report the available version.
        #[arg(long)]
        check_only: bool,
        /// Force the upgrade even if the local version is already at the latest.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, clap::Subcommand)]
pub enum SessionsAction {
    /// Show a session's wiki page. Defaults to the most recent for $PWD.
    Show {
        /// Session UUID. If omitted, returns the most recent for the cwd.
        session_id: Option<String>,
        /// Output JSON instead of Markdown.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ClaudeAction {
    /// Install the Teramind Claude plugin (`~/.claude/plugins/teramind/`).
    Install,
    /// Remove the Teramind Claude plugin. Data is untouched.
    Uninstall,
}
