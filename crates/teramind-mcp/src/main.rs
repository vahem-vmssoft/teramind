//! `teramind-mcp` binary: stdio MCP server bridging Claude Code to the
//! Teramind daemon.

use rmcp::{transport::stdio, ServiceExt};
use std::sync::Arc;
use teramind_ipc::rpc_transport::RpcTransport;
use teramind_mcp::{
    server::TeramindMcpServer, transport_https::HttpsTransport, transport_local::LocalIpcTransport,
};

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

    let transport = build_transport()?;
    let service = TeramindMcpServer::with_transport(transport)
        .serve(stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}

/// Select the RPC transport at startup:
/// - If `team.toml` exists in the config directory: use HTTPS with DPoP
///   signing, wrapped in `GrepFallback` for offline read resilience.
/// - Otherwise: use the local Unix-domain socket (daemon must be running).
fn build_transport() -> anyhow::Result<Arc<dyn RpcTransport>> {
    let config_dir = teramind_core::team::default_config_dir();
    let team_toml = config_dir.join("team.toml");

    if team_toml.exists() {
        let cfg = teramind_core::team::TeamConfig::load(&team_toml)?;
        let key = teramind_core::team::load_signing_key(&config_dir.join("team-key"))?;
        let https =
            Arc::new(HttpsTransport::new(Arc::new(cfg), Arc::new(key))) as Arc<dyn RpcTransport>;

        // Resolve raw_dir for GrepFallback using the same XDG logic as teramindd.
        let raw_dir = {
            let home = std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
            std::env::var_os("XDG_DATA_HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| home.join(".local/share"))
                .join("teramind")
                .join("raw")
        };

        let transport = Arc::new(teramind_ipc::grep_fallback_client::GrepFallback::new(
            https, raw_dir,
        )) as Arc<dyn RpcTransport>;
        Ok(transport)
    } else {
        let sock = teramind_ipc::transport::default_socket_path();
        Ok(Arc::new(LocalIpcTransport::new(sock)) as Arc<dyn RpcTransport>)
    }
}
