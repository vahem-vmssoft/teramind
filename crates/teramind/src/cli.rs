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
    /// Stream live team activity (WebSocket; requires team mode).
    Feed {
        /// Keep streaming until interrupted.
        #[arg(long)]
        follow: bool,
        /// Print recent buffered events before tailing (v1.0: best-effort, may print nothing).
        #[arg(long, default_value = "0")]
        backlog: u32,
    },
    /// Inspect skills and codifier observations.
    Skills {
        #[command(subcommand)]
        action: SkillsAction,
    },
    /// Redaction utilities.
    Redact {
        #[command(subcommand)]
        action: RedactAction,
    },
    /// Team-mode utilities (markers, share toggles).
    Team {
        #[command(subcommand)]
        action: TeamAction,
    },
}

#[derive(Debug, Subcommand)]
pub enum RedactAction {
    /// Preview redactions on the given input (sanity-check).
    /// Reads from <input> if provided, otherwise stdin.
    Test {
        /// Optional inline input. If omitted, stdin is read.
        input: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum TeamAction {
    /// Flip the per-project team-share marker in `.teramind/team-share.toml`.
    ///
    /// Exactly one of --enable or --disable must be passed.
    ShareSet {
        /// Set share=true.
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        /// Set share=false.
        #[arg(long, conflicts_with = "enable")]
        disable: bool,
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
pub enum SkillsAction {
    /// List skills.
    List {
        /// Filter: all | pending | approved | rejected | codified | authored.
        #[arg(long, default_value = "all")]
        filter: String,
        /// Maximum rows to return.
        #[arg(long, default_value = "50")]
        limit: u32,
    },
    /// Print one skill's full body.
    Show {
        /// Skill name or UUID.
        name_or_id: String,
    },
    /// List codifier observations (for debugging).
    Observations {
        /// Detector kind filter (tool_chain | problem_fix | llm_proposal).
        #[arg(long)]
        kind: Option<String>,
        /// Minimum frequency threshold.
        #[arg(long, default_value = "0")]
        min_freq: i32,
        /// Status filter (new | promoted | rejected).
        #[arg(long)]
        status: Option<String>,
        /// Maximum rows to return.
        #[arg(long, default_value = "50")]
        limit: u32,
    },
}
