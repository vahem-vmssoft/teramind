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
    if let Err(_) = spawn::ensure_daemon_connected(&socket).await {
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
    let _ = client.notify(Notify::Ingest(envelope.clone())).await;
    std::process::exit(0);
}
