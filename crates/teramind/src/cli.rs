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
    Init,
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
}

#[derive(Debug, Subcommand)]
pub enum ClaudeAction {
    /// Install the Teramind Claude plugin (`~/.claude/plugins/teramind/`).
    Install,
    /// Remove the Teramind Claude plugin. Data is untouched.
    Uninstall,
}
