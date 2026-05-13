//! `teramind-mcp` binary: stdio MCP server bridging Claude Code to the
//! Teramind daemon.

use rmcp::{ServiceExt, transport::stdio};
use teramind_mcp::server::TeramindMcpServer;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    // Logging goes to stderr only; stdio (stdin/stdout) is the MCP JSON-RPC
    // wire. We keep this minimal so a bare `teramind-mcp` invocation behaves
    // as a clean MCP transport.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let service = TeramindMcpServer::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
