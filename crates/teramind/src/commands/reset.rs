pub async fn run(purge: bool, confirm: bool) -> anyhow::Result<()> {
    if !confirm {
        anyhow::bail!("`teramind reset` will delete local data; re-run with --confirm to proceed");
    }
    let paths = teramindd::paths::Paths::resolve()?;
    for d in [&paths.pgdata_dir, &paths.raw_dir, &paths.inbox_dir, &paths.dead_letter_dir] {
        if d.exists() { std::fs::remove_dir_all(d)?; }
    }
    if purge {
        if paths.config_dir.exists() { std::fs::remove_dir_all(&paths.config_dir)?; }
    }
    println!("teramind: local data {}cleared.", if purge { "and config " } else { "" });
    Ok(())
}
