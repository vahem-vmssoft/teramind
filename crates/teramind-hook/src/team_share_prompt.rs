//! Share-prompt notice injected into the SessionStart context when team mode
//! is configured but no per-project sharing preference has been recorded yet.

use std::path::Path;

/// Returns a notice string if team mode is configured AND no `.teramind/team-share.toml`
/// marker exists anywhere in the ancestry of `cwd` up to `$HOME`. Returns `None` otherwise.
pub fn maybe_share_prompt(cwd: &Path) -> Option<String> {
    let team_toml = teramind_core::team::default_config_dir().join("team.toml");
    if !team_toml.exists() {
        return None;
    }
    let home = std::env::var("HOME")
        .ok()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/"));
    if teramind_core::team_share::find_marker(cwd, &home).is_some() {
        return None;
    }
    Some(format!(
        "⚠️ This project at `{}` has no Teramind team-sharing preference set. \
         Please ask the user once: \"Share captures from this project with the team?\" \
         Then call `mcp__teramind__team_share_set(scope: 'project', share: true | false)` \
         to record their answer. Until then, captures stay local-only.",
        cwd.display()
    ))
}
