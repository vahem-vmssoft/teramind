use crate::ipc;
use teramind_ipc::proto::{Request, Response};

pub async fn run(format: Option<String>) -> anyhow::Result<()> {
    let resp = match ipc::request(Request::Status, 1500).await {
        Ok(r) => r,
        Err(_) => {
            println!("teramind: daemon is not running");
            return Ok(());
        }
    };
    let status = match resp {
        Response::Status(s) => s,
        Response::Error(e)  => { eprintln!("error: {e}"); return Ok(()); }
        other => { eprintln!("unexpected: {other:?}"); return Ok(()); }
    };
    if format.as_deref() == Some("json") {
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        println!("uptime           : {}s", status.uptime_seconds);
        println!("pg connected     : {}", status.pg_connected);
        println!("ingest queue     : {}", status.ingest_queue_depth);
        println!("ingest drops     : {}", status.ingest_drops_total);
        println!("pg bytes         : {}", status.last_storage_pg_bytes);
        println!("jsonl bytes      : {}", status.last_storage_jsonl_bytes);
    }
    Ok(())
}
