use teramind_ipc::proto::Request;

pub async fn run() -> anyhow::Result<()> {
    super::stop::run().await?;

    // Shutdown is graceful (embedded Postgres teardown can take a few seconds);
    // start() bails "already running" if the old daemon still answers Ping, so
    // wait until it stops responding before relaunching.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        if crate::ipc::request(Request::Ping, 250).await.is_err() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    super::start::run().await
}
