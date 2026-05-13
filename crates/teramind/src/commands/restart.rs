pub async fn run() -> anyhow::Result<()> {
    super::stop::run().await?;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    super::start::run().await
}
