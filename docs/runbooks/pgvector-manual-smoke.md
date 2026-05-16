# Manual smoke: pgvector + semantic search

Confirms that the embedding worker fills vectors, semantic search returns
paraphrase hits, and `teramind doctor` surfaces provider health.

## Prereqs

- Plans A–F installed (Teramind Core).
- Ollama running on localhost:11434 with `nomic-embed-text-v2-moe` pulled:
  ```sh
  ollama pull nomic-embed-text-v2-moe
  ```
- `~/.config/teramind/search.toml` has `semantic = 0.4` under `[blend]`.

## Steps

1. Start the daemon: `teramind start`.
2. Run a Claude session that writes a few turns and edits a file.
3. Check the backlog drains:
   ```sh
   teramind doctor | grep "embedding"
   ```
   Expect: `embedding provider: ollama:nomic-embed-text-v2-moe (healthy)` and
   `embedding backlog: 0 rows (last filled <N>s ago)` within ~30 s.
4. Run a paraphrase search:
   ```sh
   teramind search "how does the access token refresh"
   ```
   Expect: a hit that wasn't previously findable via the lexical-only search.
5. Stop Ollama:
   ```sh
   killall ollama
   ```
   Re-run `teramind doctor` — expect `unhealthy`. Re-run the search —
   expect lexical-only results plus a warning in the daemon log.

## Troubleshooting

- "embedding provider: ollama … unhealthy" right after start: confirm
  `ollama serve` is up; `curl http://localhost:11434/api/version`.
- Backlog never drains: check `~/.local/share/teramind/logs/teramindd.log.*`
  for `embed_with_bisect failed` lines; verify the model is pulled.
- Paraphrase search returns nothing: confirm `search.toml` has
  `semantic = 0.4` (default is `0.0`) AND the daemon was restarted after
  the change.
