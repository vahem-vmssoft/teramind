pub async fn run() -> anyhow::Result<()> {
    println!("teramind {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}
