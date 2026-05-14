use std::path::Path;
use std::time::Duration;
use teramind_ipc::{client::{IpcClient, StreamClient}, proto::{Request, Response}, transport::connect};

fn list_cwd_files(cwd: &std::path::Path, limit: usize) -> Vec<String> {
    use ignore::WalkBuilder;
    let mut out = Vec::with_capacity(limit);
    let walker = WalkBuilder::new(cwd).hidden(false).max_depth(Some(3)).build();
    for entry in walker.flatten().take(limit * 8) {
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            if let Ok(rel) = entry.path().strip_prefix(cwd) {
                out.push(rel.to_string_lossy().to_string());
                if out.len() >= limit { break; }
            }
        }
    }
    out
}

/// Ask the daemon for an auto-recall digest. Prints the markdown to stdout if any.
/// Best-effort: any error silently no-ops. Never blocks Claude longer than `deadline`.
pub async fn run(socket: &Path, cwd: String, deadline: Duration) -> std::io::Result<()> {
    let cwd_files = list_cwd_files(std::path::Path::new(&cwd), 50);
    let result = tokio::time::timeout(deadline, async {
        let stream = connect(socket).await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let mut client = StreamClient::new(stream);
        let resp = client.request(Request::AutoRecall(teramind_core::types::AutoRecallRequest { cwd, limit: 5, cwd_files })).await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        Ok::<_, std::io::Error>(resp)
    }).await;
    match result {
        Ok(Ok(Response::AutoRecallDigest { markdown, .. })) if !markdown.is_empty() => {
            println!("{markdown}");
        }
        _ => {}
    }
    Ok(())
}
