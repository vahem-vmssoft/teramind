use anyhow::Context;
use std::path::PathBuf;

pub async fn run(
    team: bool,
    server: Option<String>,
    invite: Option<String>,
    device_name: Option<String>,
) -> anyhow::Result<()> {
    if team {
        let server =
            server.ok_or_else(|| anyhow::anyhow!("--server required with --team"))?;
        let invite =
            invite.ok_or_else(|| anyhow::anyhow!("--invite required with --team"))?;
        return crate::commands::init_team::run(server, invite, device_name).await;
    }
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
