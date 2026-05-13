use std::path::{Path, PathBuf};
use std::time::Duration;

/// If the daemon socket can be connected, returns `Ok(())`.
/// If not, spawn `teramindd` in the background and retry once.
/// Returns `Err` only if both connection attempts fail.
///
/// Honors the `TERAMIND_HOOK_NO_SPAWN` env var: if set, never attempts to spawn the daemon;
/// returns Err immediately on initial connect failure. Useful for tests that want to verify
/// inbox-fallback semantics.
pub async fn ensure_daemon_connected(socket: &Path) -> std::io::Result<()> {
    if try_connect(socket, Duration::from_millis(50)).await.is_ok() {
        return Ok(());
    }
    if std::env::var("TERAMIND_HOOK_NO_SPAWN").is_ok() {
        return Err(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "spawn disabled"));
    }
    spawn_daemon_detached()?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    try_connect(socket, Duration::from_millis(50)).await
}

async fn try_connect(socket: &Path, deadline: Duration) -> std::io::Result<()> {
    let r = tokio::time::timeout(deadline, teramind_ipc::transport::connect(socket)).await;
    match r {
        Ok(Ok(_stream)) => Ok(()),
        Ok(Err(e)) => Err(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, e.to_string())),
        Err(_) => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "connect deadline")),
    }
}

fn spawn_daemon_detached() -> std::io::Result<()> {
    let exe = which_teramindd()?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.stdin(std::process::Stdio::null())
       .stdout(std::process::Stdio::null())
       .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x00000008);
    }
    let _ = cmd.spawn()?;
    Ok(())
}

fn which_teramindd() -> std::io::Result<PathBuf> {
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            let cand = dir.join(if cfg!(windows) { "teramindd.exe" } else { "teramindd" });
            if cand.exists() { return Ok(cand); }
        }
    }
    if let Ok(out) = std::process::Command::new(if cfg!(windows) { "where" } else { "which" })
        .arg("teramindd").output() {
        if out.status.success() {
            if let Some(line) = String::from_utf8_lossy(&out.stdout).lines().next() {
                return Ok(PathBuf::from(line.trim()));
            }
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::NotFound, "teramindd not found"))
}
