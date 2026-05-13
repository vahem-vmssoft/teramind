use anyhow::Context;
use std::path::PathBuf;

pub async fn run() -> anyhow::Result<()> {
    let claude_home = claude_home()?;
    let plugin_dir = claude_home.join("plugins").join("teramind");

    let teramind_hook_bin = which_teramind_hook()?;
    let plugin_dir_str = plugin_dir.to_string_lossy().into_owned();
    let hook_bin_str = teramind_hook_bin.to_string_lossy().into_owned();

    if plugin_dir.exists() {
        std::fs::remove_dir_all(&plugin_dir)
            .with_context(|| format!("remove existing {}", plugin_dir.display()))?;
    }
    std::fs::create_dir_all(plugin_dir.join("hooks"))?;
    std::fs::create_dir_all(plugin_dir.join("skills"))?;

    let template_dir = locate_template_dir()?;
    for entry in walk_template(&template_dir) {
        let rel = entry.strip_prefix(&template_dir).unwrap();
        let dst = plugin_dir.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&dst)?;
            continue;
        }
        if entry.file_name().and_then(|n| n.to_str()) == Some(".gitkeep") {
            if let Some(parent) = dst.parent() { std::fs::create_dir_all(parent)?; }
            continue;
        }
        let bytes = std::fs::read(&entry)?;
        let text = String::from_utf8_lossy(&bytes)
            .replace("@TERAMIND_PLUGIN_DIR@", &plugin_dir_str)
            .replace("@TERAMIND_HOOK_BIN@", &hook_bin_str);
        if let Some(parent) = dst.parent() { std::fs::create_dir_all(parent)?; }
        std::fs::write(&dst, text.as_bytes())
            .with_context(|| format!("write {}", dst.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let was_exec = std::fs::metadata(&entry)?.permissions().mode() & 0o111 != 0;
            if was_exec {
                let mut perms = std::fs::metadata(&dst)?.permissions();
                perms.set_mode(perms.mode() | 0o755);
                std::fs::set_permissions(&dst, perms)?;
            }
        }
    }

    println!("Teramind plugin installed at {}", plugin_dir.display());

    // Post-install self-test of the hook binary.
    let status = std::process::Command::new(&teramind_hook_bin).arg("--selftest").status();
    match status {
        Ok(s) if s.success() => println!("teramind-hook self-test passed."),
        _ => println!("WARNING: teramind-hook self-test failed; hooks may not fire correctly."),
    }
    println!("Open Claude Code; run a session; then `teramind sessions` to confirm capture.");
    Ok(())
}

fn claude_home() -> anyhow::Result<PathBuf> {
    if let Ok(h) = std::env::var("CLAUDE_HOME") {
        return Ok(PathBuf::from(h));
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .context("HOME (or USERPROFILE on Windows) is not set")?;
    Ok(home.join(".claude"))
}

fn which_teramind_hook() -> anyhow::Result<PathBuf> {
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            let cand = dir.join(if cfg!(windows) { "teramind-hook.exe" } else { "teramind-hook" });
            if cand.exists() { return Ok(cand); }
        }
    }
    if let Ok(out) = std::process::Command::new(if cfg!(windows) { "where" } else { "which" })
        .arg("teramind-hook").output() {
        if out.status.success() {
            if let Some(line) = String::from_utf8_lossy(&out.stdout).lines().next() {
                return Ok(PathBuf::from(line.trim()));
            }
        }
    }
    anyhow::bail!("teramind-hook binary not found next to teramind or on PATH")
}

fn locate_template_dir() -> anyhow::Result<PathBuf> {
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            let cand = dir.join("plugins").join("claude");
            if cand.join("plugin.json").exists() { return Ok(cand); }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut p = cwd.clone();
        for _ in 0..6 {
            let cand = p.join("plugins").join("claude");
            if cand.join("plugin.json").exists() { return Ok(cand); }
            match p.parent() {
                Some(parent) => p = parent.to_path_buf(),
                None => break,
            }
        }
    }
    if let Ok(d) = std::env::var("TERAMIND_PLUGIN_TEMPLATE_DIR") {
        let p = PathBuf::from(d);
        if p.join("plugin.json").exists() { return Ok(p); }
    }
    anyhow::bail!("Could not locate the Claude plugin template directory; \
                   set TERAMIND_PLUGIN_TEMPLATE_DIR to the path containing plugin.json")
}

fn walk_template(root: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn walk(p: &std::path::Path, out: &mut Vec<PathBuf>) {
        if p.is_dir() {
            if let Ok(rd) = std::fs::read_dir(p) {
                for entry in rd.flatten() { walk(&entry.path(), out); }
            }
        } else {
            out.push(p.to_path_buf());
        }
    }
    walk(root, &mut out);
    out
}
