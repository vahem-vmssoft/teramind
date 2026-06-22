# Teramind Claude Plugin

This directory is the Teramind plugin, loaded by Claude Code via the marketplace
manifest at the repo root (`.claude-plugin/marketplace.json`). Claude Code
auto-discovers the components here: `hooks/hooks.json`, `.mcp.json`, and
`commands/`.

The hooks and MCP server invoke the `teramind-hook` and `teramind-mcp` binaries
**by name**, so those must be on your `PATH`. See the repo root `README.md` for
install and the `/plugin marketplace add` flow.
