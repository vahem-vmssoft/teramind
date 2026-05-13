use teramind_ipc::proto::{Request, Response};

pub async fn run() -> anyhow::Result<()> {
    println!("teramind doctor");
    let paths = teramindd::paths::Paths::resolve()?;
    let pid = if paths.pid_file.exists() {
        std::fs::read_to_string(&paths.pid_file).ok().map(|s| s.trim().to_string())
    } else { None };
    println!("  pid file       : {} ({})", paths.pid_file.display(), pid.as_deref().unwrap_or("missing"));
    println!("  socket         : {} ({})", paths.socket_path.display(),
        if paths.socket_path.exists() { "present" } else { "absent" });
    println!("  data dir       : {}", paths.data_dir.display());
    println!("  config dir     : {}", paths.config_dir.display());
    println!("  dead_letter    : {} files", dir_count(&paths.dead_letter_dir)?);
    println!("  inbox          : {} files", dir_count(&paths.inbox_dir)?);
    match crate::ipc::request(Request::Status, 1500).await {
        Ok(Response::Status(s)) => {
            println!("  daemon         : up ({}s uptime)", s.uptime_seconds);
            println!("  ingest queue   : {}", s.ingest_queue_depth);
            println!("  ingest drops   : {}", s.ingest_drops_total);
            println!("  pg bytes       : {}", s.last_storage_pg_bytes);
            println!("  jsonl bytes    : {}", s.last_storage_jsonl_bytes);
        }
        Ok(other) => println!("  daemon         : unexpected response {:?}", other),
        Err(_)     => println!("  daemon         : not responding"),
    }
    Ok(())
}

fn dir_count(p: &std::path::Path) -> anyhow::Result<usize> {
    if !p.exists() { return Ok(0); }
    Ok(std::fs::read_dir(p)?.filter_map(Result::ok).count())
}
