use anyhow::Context;
use std::path::PathBuf;

pub async fn run() -> anyhow::Result<()> {
    let paths = teramindd::paths::Paths::resolve()?;
    paths.ensure_dirs()?;

    let cfg_path: PathBuf = paths.config_dir.join("config.toml");
    if !cfg_path.exists() {
        let default = include_str!("../../../../crates/teramindd/src/default_config.toml");
        std::fs::write(&cfg_path, default).context("write default config")?;
    }
    println!("Teramind initialized.");
    println!("  data dir   : {}", paths.data_dir.display());
    println!("  config dir : {}", paths.config_dir.display());
    println!("  socket     : {}", paths.socket_path.display());
    Ok(())
}
