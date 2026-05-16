use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "teramind-sync-server", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print version and exit.
    Version,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Version => {
            println!("teramind-sync-server {}", teramind_sync_server::VERSION);
            Ok(())
        }
    }
}
