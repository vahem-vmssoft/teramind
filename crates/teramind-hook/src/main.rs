use std::io::Read;
use std::sync::Arc;
use teramind_hook::{hook_input::HookInput, inbox, spawn, translate};
use teramind_ipc::{
    client::{IpcClient, StreamClient},
    proto::Notify,
    rpc_transport::RpcTransport,
    transport::{connect, default_socket_path},
};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--selftest") {
        match teramind_hook::selftest::run() {
            Ok(()) => std::process::exit(0),
            Err(e) => {
                eprintln!("teramind-hook selftest FAILED: {e}");
                std::process::exit(1);
            }
        }
    }
    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_err() {
        std::process::exit(0);
    }
    let parsed: HookInput = match serde_json::from_str(&buf) {
        Ok(p) => p,
        Err(_) => std::process::exit(0),
    };
    let envelope = match translate::translate(parsed) {
        Some(e) => e,
        None => std::process::exit(0),
    };

    let socket = default_socket_path();
    if spawn::ensure_daemon_connected(&socket).await.is_err() {
        let _ = inbox::write_envelope(&envelope);
        std::process::exit(0);
    }
    let stream = match connect(&socket).await {
        Ok(s) => s,
        Err(_) => {
            let _ = inbox::write_envelope(&envelope);
            std::process::exit(0);
        }
    };
    let mut client = StreamClient::new(stream);
    let is_session_start = matches!(
        envelope.event,
        teramind_core::types::ingest_event::IngestEvent::SessionStart { .. }
    );
    let session_cwd = match &envelope.event {
        teramind_core::types::ingest_event::IngestEvent::SessionStart { cwd, .. } => {
            Some(cwd.clone())
        }
        _ => None,
    };
    let _ = client.notify(Notify::Ingest(envelope.clone())).await;
    drop(client);

    if is_session_start {
        if let Some(cwd) = session_cwd {
            // Build the RPC transport (local IPC in local-first mode, HTTPS+GrepFallback in team mode).
            let transport = match build_transport() {
                Ok(t) => t,
                Err(_) => std::process::exit(0),
            };
            // Best-effort auto-recall — cap at 2s so we don't block Claude.
            let _ = teramind_hook::auto_recall::run(
                transport,
                cwd.clone(),
                std::time::Duration::from_secs(2),
            )
            .await;
            // Inject team-share prompt if team mode is configured but no marker set.
            if let Some(notice) =
                teramind_hook::team_share_prompt::maybe_share_prompt(std::path::Path::new(&cwd))
            {
                println!("{notice}");
            }
        }
    }

    std::process::exit(0);
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
        let https = Arc::new(teramind_mcp::transport_https::HttpsTransport::new(
            Arc::new(cfg),
            Arc::new(key),
        )) as Arc<dyn RpcTransport>;

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

        let transport =
            Arc::new(teramind_ipc::grep_fallback_client::GrepFallback::new(https, raw_dir))
                as Arc<dyn RpcTransport>;
        Ok(transport)
    } else {
        let sock = teramind_ipc::transport::default_socket_path();
        Ok(Arc::new(teramind_mcp::transport_local::LocalIpcTransport::new(sock))
            as Arc<dyn RpcTransport>)
    }
}
