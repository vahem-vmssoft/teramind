# Manual smoke: FS watcher + per-turn diff capture

Verifies that real Claude Code edits land in `file_diffs` with the right
attribution, and that auto-recall surfaces them on the next session.

## Prereqs

- `teramind` + `teramindd` installed at `~/.local/share/teramind/bin/`.
- `teramind init && teramind start && teramind claude install` already run.
- Working directory is a real git repo.

## Steps

1. Open Claude Code in your project directory.
2. Ask the agent to `Edit` a small file (e.g. add a comment to `README.md`).
3. After the agent reports the edit is done, run from another terminal:

   ```sh
   psql "postgresql:///teramind" -c \
     "SELECT rel_path, attribution, length(unified_diff) AS diff_size, captured_at \
      FROM file_diffs ORDER BY captured_at DESC LIMIT 5;"
   ```

   - **Expect:** A row with `rel_path` matching what the agent edited, `attribution = 'agent'`, non-zero `diff_size`.

4. From the same terminal (NOT in Claude Code), manually edit another file:

   ```sh
   echo "" >> README.md
   ```

5. Re-run the SQL above.

   - **Expect:** A new row with `attribution = 'human'` and `turn_id = NULL`.

6. Stop Claude Code, then start a new session in the same directory.

   - **Expect:** The SessionStart auto-recall digest printed to Claude's
     stdout includes a "Recent diffs in this project" section listing
     the files from steps 2 and 4.

7. Check `teramind status --format=json` for `fs_watcher_gaps_total`.

   - **Expect:** `0` in a normal session.

## Troubleshooting

- No rows appear after step 3: check `~/.local/share/teramind/logs/teramindd.log.*`
  for `fs_watcher` warnings. Common cause: cwd is on a filesystem that
  doesn't support inotify (NFS, some Docker bind mounts).
- All rows show `attribution = 'human'`: the write-tool ring isn't being
  populated. Check that `ToolCallEnd` events include `tool_name` — should
  be visible in the JSONL shadow log under `~/.local/share/teramind/raw/`.
- Excerpts contain secrets: file a bug. Redaction must run before persist.
