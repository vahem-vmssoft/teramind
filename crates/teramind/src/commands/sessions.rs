//! `teramind sessions show [<id>] [--json]`

use teramind_ipc::proto::{Request, Response};

pub async fn show(session_id: Option<String>, json: bool) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let req = Request::WikiLookup {
        session_id,
        cwd: Some(cwd),
    };

    let resp = crate::ipc::request(req, 10_000).await?;
    match resp {
        Response::WikiPage {
            session_id,
            cwd,
            model,
            content,
            generated_at,
        } => {
            if json {
                let body = serde_json::json!({
                    "session_id": session_id,
                    "cwd": cwd,
                    "model": model,
                    "content": content,
                    "generated_at": generated_at.to_string(),
                });
                println!("{}", serde_json::to_string_pretty(&body)?);
            } else {
                println!("{}", content);
            }
        }
        Response::WikiNotFound => {
            eprintln!("teramind: no wiki page found for the given criteria.");
            eprintln!("Run `teramind doctor` for summarizer health.");
            std::process::exit(2);
        }
        Response::Error(msg) => anyhow::bail!("wiki lookup failed: {msg}"),
        other => anyhow::bail!("unexpected response: {other:?}"),
    }
    Ok(())
}
