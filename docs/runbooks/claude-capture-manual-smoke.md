# Manual smoke: Claude Code → Teramind capture

This runbook verifies the Plan B integration end-to-end with a real Claude Code session.
It is run before tagging a Plan B release and after any change to `teramind-hook` or
`teramind claude install`.

## Prerequisites

- Claude Code installed and authenticated (`claude --version` works).
- Teramind binaries built: `cargo build --workspace --release`.
- `~/.local/bin/teramind` and `~/.local/bin/teramind-hook` either symlinked from `target/release/`
  or installed via the (future Plan E) installer.

## Procedure

1. Initialize state and install the plugin:

   ```bash
   teramind init
   teramind start
   teramind status   # should show uptime > 0
   teramind claude install
   ```

   Expect: `Teramind plugin installed at /Users/<you>/.claude/plugins/teramind`
   followed by `teramind-hook self-test passed.`

2. Open Claude Code in a scratch directory:

   ```bash
   mkdir /tmp/teramind-smoke && cd /tmp/teramind-smoke
   claude
   ```

3. Inside Claude, do at least the following:
   - Type a prompt that produces an assistant response (e.g., "Hello, who are you?").
   - Ask Claude to use a tool (e.g., "Read `/etc/hosts` and tell me the first line.").
   - Exit (Ctrl-D or `/exit`).

4. From your shell, verify capture:

   ```bash
   teramind sessions --last 5
   ```

   Expect: at least one row for the just-ended session, with non-zero turn count and tool-call count.

5. Spot-check the database directly (psql-level optional):

   ```bash
   psql "$(teramind status --format=json | jq -r '.pg_url')" -c \
     "SELECT id, cwd, started_at, ended_at FROM sessions ORDER BY started_at DESC LIMIT 1;"
   psql … -c "SELECT ordinal, length(user_prompt), length(assistant_text) FROM turns WHERE session_id = '<id>' ORDER BY ordinal;"
   psql … -c "SELECT name, length(output), is_error FROM tool_calls tc JOIN turns t ON tc.turn_id=t.id WHERE t.session_id='<id>' ORDER BY tc.ordinal;"
   ```

6. Uninstall and re-verify:

   ```bash
   teramind claude uninstall
   ls ~/.claude/plugins/teramind   # should error: no such file
   ```

   Open Claude Code again, type a prompt, exit. Verify no new session row appeared:

   ```bash
   teramind sessions --last 5    # should be unchanged from step 4
   ```

## Failure modes

| Symptom | Likely cause | Fix |
|---|---|---|
| `teramind sessions` shows no new session after step 3 | Plugin hooks didn't fire | Check `~/.claude/plugins/teramind/plugin.json` for absolute paths; rerun `teramind claude install`. |
| Session row present but no turns | `UserPromptSubmit` hook silently failed | Check that `teramind-hook` runs `--selftest` cleanly. Inspect `~/.local/share/teramind/inbox/` for stranded events. |
| Tool calls present but `output` is empty | Claude version uses `tool_output` instead of `tool_response` | Already aliased via serde; if not, file an issue with Claude's hook JSON sample. |
| Hook binary not found by Claude | Absolute path in `plugin.json` is wrong | Reinstall plugin via `teramind claude install`. |

## When to re-run this runbook

- Every minor version bump of Claude Code (their hook payload shapes have evolved historically).
- Every change to `teramind-hook` translate logic.
- Before merging any PR that touches `plugins/claude/` or `teramind claude install`.
