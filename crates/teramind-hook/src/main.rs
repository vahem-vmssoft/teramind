use std::io::Read;
use teramind_hook::{hook_input::HookInput, inbox, spawn, translate};
use teramind_ipc::{client::{IpcClient, StreamClient}, proto::Notify, transport::{connect, default_socket_path}};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--selftest") {
        match teramind_hook::selftest::run() {
            Ok(()) => std::process::exit(0),
            Err(e) => { eprintln!("teramind-hook selftest FAILED: {e}"); std::process::exit(1); }
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
    let is_session_start = matches!(envelope.event, teramind_core::types::ingest_event::IngestEvent::SessionStart { .. });
    let session_cwd = match &envelope.event {
        teramind_core::types::ingest_event::IngestEvent::SessionStart { cwd, .. } => Some(cwd.clone()),
        _ => None,
    };
    let _ = client.notify(Notify::Ingest(envelope.clone())).await;
    drop(client);

    if is_session_start {
        if let Some(cwd) = session_cwd {
            // Best-effort auto-recall — cap at 2s so we don't block Claude.
            let _ = teramind_hook::auto_recall::run(&socket, cwd, std::time::Duration::from_secs(2)).await;
        }
    }

    std::process::exit(0);
}
