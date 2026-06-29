pub async fn run() -> anyhow::Result<()> {
    super::stop::run().await?;

    // Poll the PID file (not the socket). The daemon removes the socket first,
    // then shuts down Postgres, then removes the PID file last — so PID-file
    // absence is the reliable signal that the port is free to reuse.
    let pid_file = teramindd::paths::Paths::resolve()?.pid_file;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while pid_file.exists() && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    super::start::run().await
}
