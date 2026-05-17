//! Integration test: `team_share_set` MCP tool dispatches
//! `Request::TeamShareSet` to the daemon over IPC.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use tempfile::tempdir;
use teramind_ipc::proto::{Notify, Request, Response};
use teramind_ipc::server::{serve_connection, IpcServer};
use teramind_ipc::transport::listen;

fn cargo_bin(name: &str) -> std::path::PathBuf {
    std::env::var(format!("CARGO_BIN_EXE_{name}"))
        .map(Into::into)
        .unwrap_or_else(|_| {
            let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
            let profile = if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            };
            std::path::PathBuf::from(target).join(profile).join(name)
        })
}

/// Minimal mock IPC server that records all requests and replies `Ok`.
#[derive(Clone)]
struct RecordingHandler {
    recorded: Arc<Mutex<Vec<Request>>>,
}

#[async_trait::async_trait]
impl IpcServer for RecordingHandler {
    async fn handle_request(&self, req: Request) -> Response {
        self.recorded.lock().unwrap().push(req);
        Response::Ok
    }
    async fn handle_notify(&self, _n: Notify) {}
}

/// Run the blocking subprocess I/O and return the tools/call response line.
fn drive_mcp_subprocess(sock_path: std::path::PathBuf) -> String {
    // Build the binary first (no-op if already fresh).
    let _ = Command::new("cargo")
        .args(["build", "-p", "teramind-mcp"])
        .status();

    let mcp = cargo_bin("teramind-mcp");
    let mut child = Command::new(&mcp)
        .env("TERAMIND_SOCKET", sock_path.to_string_lossy().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let stdin = child.stdin.as_mut().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);

    // 1. initialize
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#;
    writeln!(stdin, "{init}").unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains("\"result\""), "initialize failed: {line}");

    // 2. notifications/initialized (required by MCP protocol before tool calls)
    let initialized = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    writeln!(stdin, "{initialized}").unwrap();

    // 3. tools/call team_share_set
    let call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"team_share_set","arguments":{"scope":"project","share":true}}}"#;
    writeln!(stdin, "{call}").unwrap();

    // Read lines until we get the id=2 response or EOF.
    let mut response_line = String::new();
    loop {
        let mut l = String::new();
        let n = reader.read_line(&mut l).unwrap();
        if n == 0 {
            break;
        }
        let trimmed = l.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.contains("\"id\":2") {
            response_line = trimmed.to_owned();
            break;
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    response_line
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_share_set_dispatches_ipc_request() {
    let tmp = tempdir().unwrap();
    let sock_path = tmp.path().join("mock.sock");

    let recorded: Arc<Mutex<Vec<Request>>> = Arc::new(Mutex::new(Vec::new()));
    let listener = listen(&sock_path).unwrap();
    let handler = Arc::new(RecordingHandler {
        recorded: recorded.clone(),
    });

    // Accept exactly one connection in a background task.
    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let _ = serve_connection(stream, handler).await;
        }
    });

    // Drive the subprocess on a blocking thread so the tokio executor
    // can continue polling the mock server accept task.
    let sock_path_clone = sock_path.clone();
    let response_line = tokio::task::spawn_blocking(move || drive_mcp_subprocess(sock_path_clone))
        .await
        .unwrap();

    // Verify the MCP response contains the tool result.
    assert!(!response_line.is_empty(), "no response for tools/call id=2");
    assert!(
        response_line.contains("\"result\""),
        "expected result in response: {response_line}"
    );

    // Verify the mock server received a TeamShareSet request.
    let requests = recorded.lock().unwrap();
    assert!(!requests.is_empty(), "mock IPC server received no requests");
    let found = requests.iter().any(|r| {
        matches!(
            r,
            Request::TeamShareSet {
                scope,
                share: true,
                ..
            } if scope == "project"
        )
    });
    assert!(
        found,
        "expected TeamShareSet(scope=project, share=true); got: {requests:?}"
    );
}
