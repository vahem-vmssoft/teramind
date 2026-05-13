use std::process::Command;

pub async fn run() -> anyhow::Result<()> {
    if crate::ipc::request(teramind_ipc::proto::Request::Ping, 250).await.is_ok() {
        println!("teramind: daemon already running");
        return Ok(());
    }
    let exe = which_teramindd()?;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let _ = Command::new(&exe)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0)
            .spawn()?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new(&exe)
            .creation_flags(0x00000008)
            .spawn()?;
    }
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if crate::ipc::request(teramind_ipc::proto::Request::Ping, 250).await.is_ok() {
            println!("teramind: daemon started");
            return Ok(());
        }
    }
    anyhow::bail!("daemon spawned but did not become responsive within 5 seconds");
}

fn which_teramindd() -> anyhow::Result<std::path::PathBuf> {
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            let candidate = dir.join(if cfg!(windows) { "teramindd.exe" } else { "teramindd" });
            if candidate.exists() { return Ok(candidate); }
        }
    }
    if let Ok(out) = std::process::Command::new(if cfg!(windows) { "where" } else { "which" }).arg("teramindd").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            if let Some(line) = s.lines().next() { return Ok(line.trim().into()); }
        }
    }
    anyhow::bail!("teramindd binary not found next to teramind or on PATH")
}
