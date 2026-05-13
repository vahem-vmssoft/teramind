pub async fn install() -> anyhow::Result<()> {
    crate::commands::claude_install::run().await
}
pub async fn uninstall() -> anyhow::Result<()> {
    crate::commands::claude_uninstall::run().await
}
