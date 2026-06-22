//! Sanity checks for the committed marketplace plugin manifests.
//!
//! Claude Code loads the plugin from static JSON (no install step on our side),
//! so the only failure modes left are: a JSON typo, or a binary name that drifts
//! from what the workspace actually builds. Both make the plugin silently fail to
//! load. This test catches both; it does not exercise any runtime.
use serde_json::Value;
use std::path::{Path, PathBuf};

// Binary names the plugin invokes by PATH name. Must match `[[bin]]` names in the
// workspace (teramind-hook, teramind-mcp). Bump here if a bin is renamed.
const HOOK_BIN: &str = "teramind-hook";
const MCP_BIN: &str = "teramind-mcp";

fn repo_root() -> PathBuf {
    std::env::current_dir()
        .unwrap()
        .ancestors()
        .find(|p| p.join(".claude-plugin/marketplace.json").exists())
        .expect("could not find .claude-plugin/marketplace.json in ancestors")
        .to_path_buf()
}

fn parse(path: &Path) -> Value {
    let body = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    serde_json::from_str(&body).unwrap_or_else(|e| panic!("invalid JSON in {path:?}: {e}"))
}

#[test]
fn manifests_valid_and_consistent() {
    let root = repo_root();

    // marketplace.json -> plugin source dir resolves and names match.
    let market = parse(&root.join(".claude-plugin/marketplace.json"));
    let plugin = &market["plugins"][0];
    let source = plugin["source"].as_str().expect("plugin.source missing");
    let plugin_dir = root.join(source.trim_start_matches("./"));
    let plugin_json = parse(&plugin_dir.join(".claude-plugin/plugin.json"));
    assert_eq!(
        plugin["name"], plugin_json["name"],
        "marketplace plugin name != plugin.json name"
    );

    // hooks.json: every hook command invokes the hook binary.
    let hooks = parse(&plugin_dir.join("hooks/hooks.json"));
    let hooks_raw = std::fs::read_to_string(plugin_dir.join("hooks/hooks.json")).unwrap();
    assert!(
        hooks["hooks"].is_object(),
        "hooks.json missing top-level hooks object"
    );
    assert!(
        hooks_raw.contains(HOOK_BIN),
        "hooks.json never references {HOOK_BIN}"
    );

    // .mcp.json: the MCP server command is the mcp binary.
    let mcp = parse(&plugin_dir.join(".mcp.json"));
    assert_eq!(
        mcp["mcpServers"]["teramind"]["command"], MCP_BIN,
        "MCP command is not {MCP_BIN}"
    );
}
