# Manual smoke: session summarizer

Confirms that the summarizer worker writes wiki pages, the MCP tool returns
them, the CLI `sessions show` prints them, and `traces_fts` surfaces them
to `teramind search`.

## Prereqs

- Plans A–G installed.
- Ollama running on localhost:11434 with `qwen3.6:latest` pulled:
  ```sh
  ollama pull qwen3.6:latest
  ```

## Steps

1. Start the daemon: `teramind start`.
2. Run a Claude session in a project directory; aim for 5+ turns and >1 min wall time. End the session (close Claude Code or wait for idle).
3. Wait ~90s, then check the backlog drains:
   ```sh
   teramind doctor | grep "summary"
   ```
   Expect: `summary provider: ollama:qwen3.6:latest (healthy)` and `summary backlog: 0 sessions queued`.
4. Print the wiki page:
   ```sh
   teramind sessions show
   ```
   Expect: Markdown with `# Summary`, `# Files changed`, `# Decisions & gotchas`, `# Follow-ups` sections.
5. Search for a token from the summary:
   ```sh
   teramind search "<unique word from your session>"
   ```
   Expect: at least one hit; if it's a wiki hit, the hit type will say so.
6. Open a NEW Claude Code session in the same cwd. The SessionStart digest should include `## Most recent session summary` with a truncated wiki body.
7. Stop Ollama (`killall ollama`); rerun `teramind doctor`. Expect `unhealthy` and a paused backlog.

## Troubleshooting

- "summary provider: ollama:... (unhealthy)" right after start: confirm
  `ollama serve` is up; `curl http://localhost:11434/api/version`.
- Backlog never drains: check `~/.local/share/teramind/logs/teramindd.log.*`
  for `model not found` or `summarize failed` lines; verify `qwen3.6:latest`
  is pulled.
- `teramind sessions show` says "no wiki page found": session may have been
  too short (default min_turns=3, min_duration_secs=60). Inspect with
  `psql -c "SELECT count(*) FROM sessions_to_summarize"` to see candidates.
