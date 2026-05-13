use anyhow::Context;
use std::path::PathBuf;

pub async fn run() -> anyhow::Result<()> {
    let claude_home = claude_home()?;
    let plugin_dir = claude_home.join("plugins").join("teramind");
    if plugin_dir.exists() {
        std::fs::remove_dir_all(&plugin_dir)
            .with_context(|| format!("remove {}", plugin_dir.display()))?;
        println!("Teramind plugin removed from {}", plugin_dir.display());
    } else {
        println!("Teramind plugin was not installed (nothing at {})", plugin_dir.display());
    }
    println!("User data is untouched. Use `teramind uninstall --purge --confirm` to remove data.");
    Ok(())
}

fn claude_home() -> anyhow::Result<PathBuf> {
    if let Ok(h) = std::env::var("CLAUDE_HOME") { return Ok(PathBuf::from(h)); }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .context("HOME (or USERPROFILE on Windows) is not set")?;
    Ok(home.join(".claude"))
}
