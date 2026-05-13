# Manual smoke: Teramind search surfaces with real Claude Code

This runbook verifies Plan C's four search surfaces against a real Claude Code session.

## Prerequisites

- Plan A + B + C merged and `cargo build --release`'d.
- `teramind init && teramind start && teramind claude install` complete.
- At least one prior Claude session's worth of captured traces in Postgres.

## Procedure

### 1. CLI search

```bash
teramind search "<a topic from a prior session>"
```

Expect: ranked hits, with snippets, scores, and timestamps. The `(N hits in M ms)` line should appear on stderr.

### 2. MCP tool from inside Claude

```bash
cd /tmp/teramind-search-smoke && claude
```

Inside Claude, ask: *"Use the teramind search tool to find anything we've talked about regarding `<topic>`. List the top 3 hits."*

Expect: Claude calls `mcp__teramind__search` (visible in tool-use indicator), receives structured Hit JSON, summarizes the hits.

### 3. Slash command

Inside Claude:

```
/teramind:search <topic>
```

Expect: equivalent to (2) but triggered by user explicitly.

### 4. Auto-recall on SessionStart

Open Claude in a directory where prior traces exist:

```bash
cd /path/with/history && claude
```

Expect: Claude's first response acknowledges or references the auto-injected "Recent Teramind context" digest from prior sessions.

### 5. Grep fallback

Stop Postgres only (kill the embedded PG child but leave teramindd running) and rerun (1):

```bash
# In a separate shell, find the PG pid and kill it
pkill -f postgres
teramind search "anything"
```

Expect: the `(degraded: Postgres unreachable …)` banner on stderr, plus best-effort hits from JSONL.

Then `teramind restart` to recover.

## Failure modes

| Symptom | Likely cause | Fix |
|---|---|---|
| CLI search returns 0 hits but you know prior traces exist | The 30s MV refresh hasn't fired yet | Wait 30 s; the daemon's scheduler will refresh `traces_fts`. Verify with `teramind status`. |
| MCP tool not visible to Claude | `.mcp.json` not patched at install time, or `teramind-mcp` not on PATH | Reinstall plugin with `teramind claude install`. |
| Slash command not visible | Plugin's `commands/` dir missing from `~/.claude/plugins/teramind/` | Reinstall plugin. |
| Auto-recall digest missing | Hook timed out or daemon AutoRecall failed | Check `~/.local/share/teramind/logs/`. Increase the 2s budget in `auto_recall.rs` if needed. |

## When to re-run this runbook

- Every change to `crates/teramindd/src/services/search.rs` or `grep_fallback.rs`.
- Every change to `teramind-mcp` tool definitions.
- Every Claude Code minor version (MCP and hook payload formats have evolved historically).
