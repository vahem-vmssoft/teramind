use std::path::Path;
use std::time::Duration;
use teramind_ipc::{client::{IpcClient, StreamClient}, proto::{Request, Response}, transport::connect};

/// Ask the daemon for an auto-recall digest. Prints the markdown to stdout if any.
/// Best-effort: any error silently no-ops. Never blocks Claude longer than `deadline`.
pub async fn run(socket: &Path, cwd: String, deadline: Duration) -> std::io::Result<()> {
    let result = tokio::time::timeout(deadline, async {
        let stream = connect(socket).await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let mut client = StreamClient::new(stream);
        let resp = client.request(Request::AutoRecall(teramind_core::types::AutoRecallRequest { cwd, limit: 5 })).await
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
