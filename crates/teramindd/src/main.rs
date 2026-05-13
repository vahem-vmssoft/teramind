use teramindd::app::App;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    App::run().await
}
