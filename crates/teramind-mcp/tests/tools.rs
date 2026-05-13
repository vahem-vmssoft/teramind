#![cfg(unix)]
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use tempfile::tempdir;

fn cargo_bin(name: &str) -> std::path::PathBuf {
    std::env::var(format!("CARGO_BIN_EXE_{name}")).map(Into::into)
        .unwrap_or_else(|_| {
            let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
            let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
            std::path::PathBuf::from(target).join(profile).join(name)
        })
}

#[test]
fn mcp_server_responds_to_initialize() {
    let _ = Command::new("cargo").args(["build", "-p", "teramind-mcp"]).status();
    let tmp = tempdir().unwrap();
    let mcp = cargo_bin("teramind-mcp");

    let mut child = Command::new(&mcp)
        .env("TERAMIND_SOCKET", tmp.path().join("no-daemon.sock"))
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
        .spawn().unwrap();

    let stdin = child.stdin.as_mut().unwrap();
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#;
    writeln!(stdin, "{init}").unwrap();

    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let read = reader.read_line(&mut line).unwrap();
    assert!(read > 0, "expected response line");
    assert!(line.contains("\"result\""), "expected initialize result: {line}");

    let _ = child.kill();
}
