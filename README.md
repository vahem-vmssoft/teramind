# Teramind

A knowledge layer for AI coding agents. Teramind captures your Claude Code
sessions — prompts, tool calls, file diffs — into a local store and makes them
searchable later, by you and by the agent itself mid-session. Local-first by
default; opt-in team sync.

See [docs/PR-FAQ.md](docs/PR-FAQ.md) for the full product story and
[docs/ROADMAP.md](docs/ROADMAP.md) for what's shipped vs. planned.

## Prerequisites

- **Claude Code** — installed and authenticated (`claude --version` works).
- **A Rust toolchain** — only needed to build from source (see below). `rustup`
  installs it; the version is pinned by `rust-toolchain.toml`.
- **Internet on first run** — the daemon downloads a private embedded PostgreSQL
  the first time it starts (no system Postgres install required). If you enable
  the local embedding model (see *Configure semantic search* below) it downloads
  ~a few hundred MB on first use.
- **Disk** — captured history grows modestly (a heavy user accumulates a few GB
  per year).

Supported platforms: Linux and macOS (x86_64 / aarch64). Windows binaries build
but ship unsigned.

## Install

### Option A — released binary (recommended once releases are published)

```sh
curl -fsSL https://get.teramind.dev/install.sh | sh
```

This downloads the release archive for your platform, verifies its SHA-256,
extracts all binaries to `~/.local/share/teramind/bin/`, and symlinks `teramind`
into `~/.local/bin/`. See `installer/install.sh` (Unix) / `installer/install.ps1`
(Windows), or `installer/homebrew/` for the Homebrew formula.

### Option B — build from source

```sh
git clone <this-repo> teramind && cd teramind
cargo build --workspace --release

# Put the binaries somewhere stable on PATH (NOT target/release directly —
# a later `cargo clean` would break the absolute paths baked in at install).
mkdir -p ~/.local/bin
for b in teramind teramind-hook teramind-mcp teramindd; do
  ln -sf "$PWD/target/release/$b" ~/.local/bin/"$b"
done
```

All four binaries must be on your `PATH`: the plugin invokes `teramind-hook` and
`teramind-mcp` by name, and `teramind-hook` auto-spawns `teramindd` (which it
finds next to itself). Keeping them co-located on `PATH`, as above, satisfies
both.

## Set up the plugin

Teramind ships as a Claude Code **marketplace plugin**. With the binaries on
`PATH` (above), add the marketplace and install it from inside Claude Code:

```
/plugin marketplace add vahem-vmssoft/teramind
/plugin install teramind@teramind
```

Then restart Claude Code (or run `/reload-plugins`) so a new session loads the
hooks. **That's the whole setup** — no `teramind init`, no `teramind start`
required: the first session's hook auto-spawns the daemon, which creates its data
dir, starts the embedded Postgres, and runs migrations on its own.

Verify the plugin is wired up:

```sh
claude plugin list                  # teramind@teramind should be listed/enabled
claude plugin details teramind      # expect: 6 hooks + 1 MCP server
```

Notes:
- The plugin only loads at Claude **startup**, so it applies to *new* sessions.
- A teammate must have the binaries on `PATH` first — the plugin loads without
  them but captures nothing (it calls `teramind-hook`/`teramind-mcp` by name).
- *Developing locally?* Add the marketplace from a path instead of GitHub:
  `claude plugin marketplace add /path/to/teramind` — same files, no push needed.

## Configure semantic search (optional, recommended)

Out of the box, capture and **keyword** search work with no extra setup. Two
things are *off by default* and need config files in `~/.config/teramind/`:

**1. The embedding model.** The default points at [Ollama](https://ollama.com);
without it running, `teramind doctor` shows `embedding ... (unhealthy)` and no
embeddings are generated. To run a model fully in-process (no external service),
create `~/.config/teramind/embed.toml`:

```toml
provider = "fastembed"          # in-process; downloads its model on first start
model    = "nomic-embed-text-v1.5"
```

**2. The search blend.** Even with embeddings generated, search ignores them
until you give the semantic signal a non-zero weight. Create
`~/.config/teramind/search.toml`:

```toml
[blend]
fts      = 0.6
semantic = 0.5                  # 0.0 = keyword-only (the default)
```

Restart the daemon to load either file (see *Restarting* under Troubleshooting),
then confirm: `teramind status --format=json` should show
`"embedding_healthy": true`. A query with no shared words now finds related work:

```sh
teramind search "send a file to cloud storage"   # matches a turn about uploading
```

## Configure session summaries (optional)

End-of-session summaries need a chat model — either local Ollama or a cloud key.
There is **no in-process summarizer** (unlike embeddings). Without one, Teramind
still captures and searches; it just skips the summary. Pick one path and run it.

### Option A — local (Ollama, nothing leaves your machine)

```sh
# install Ollama (if you don't have it) and pull a chat model
curl -fsSL https://ollama.com/install.sh | sh
ollama pull qwen2.5:3b           # 3B suits laptop CPUs (no GPU); use :7b if you have a GPU

# point Teramind at it
cat > ~/.config/teramind/summarize.toml <<'TOML'
provider = "ollama"
model    = "qwen2.5:3b"          # must match what you pulled
# ollama url defaults to http://localhost:11434
TOML
```

### Option B — cloud (Anthropic; faster to set up, sends redacted content out)

```sh
cat > ~/.config/teramind/summarize.toml <<'TOML'
provider       = "anthropic"
model          = "claude-haiku-4-5-20251001"   # cheap/fast; fine for summaries
network_egress = true                          # required for any cloud provider
TOML

# API key in a 0600 secrets file (the daemon refuses looser permissions)
printf 'anthropic_api_key = "sk-ant-REPLACE_ME"\n' > ~/.config/teramind/secrets.toml
chmod 600 ~/.config/teramind/secrets.toml
```

(OpenAI is a valid `provider` value but its HTTPS wiring is deferred — use
`anthropic` for cloud. Redaction strips secrets before any egress.)

### Apply and verify (either option)

```sh
kill -TERM "$(cat ~/.local/share/teramind/teramindd.pid)" && teramind start
teramind status --format=json | grep summary_healthy        # want: true
```

Summaries are gated and asynchronous: they fire only for sessions with **≥3
turns** and **≥60s** duration, and appear within ~30s of the session ending. A
trivial test session won't produce one. View the latest:

```sh
teramind sessions show       # the summary ("wiki page") for the most recent session in $PWD
```

## Verify

Open a Claude Code session, send a prompt, use a tool, exit, then:

```sh
teramind doctor                            # health report: daemon up, 0 dead-letters
teramind search "<a word you typed>" --grep  # --grep reads the raw log (always live)
teramind sessions show                     # the session's summary (needs a summarizer)
```

For a deeper look at exactly what was captured, the raw redacted event log is the
quickest source (no DB needed):

```sh
jq -r 'select(.event) | .event.type' ~/.local/share/teramind/raw/$(date +%F).jsonl | sort | uniq -c
```

You should see `session_start`, `user_prompt`, `tool_call_start/end`, and
`session_end`. Note: plain `teramind search` uses the `traces_fts` index, which
is refreshed on a timer — a *just*-captured turn may not appear there for a few
seconds, which is why `--grep` (live over the raw log) is the reliable
immediate check. The full checklist is in
[docs/runbooks/claude-capture-manual-smoke.md](docs/runbooks/claude-capture-manual-smoke.md).

## Troubleshooting

**Restarting / config changes don't take effect.** `teramind stop` and
`teramind restart` send a shutdown request that the daemon may not act on — the
process can keep running, so a new config file won't load. Restart reliably by
signalling the PID directly:

```sh
kill -TERM "$(cat ~/.local/share/teramind/teramindd.pid)" && teramind start
```

**`embedding ... (unhealthy)` in `teramind doctor`.** The default embedding
provider is Ollama and it isn't running. Either start Ollama, or switch to the
in-process model — see *Configure semantic search* above.

**Search returns nothing for a word you know you typed.** Three usual causes:
the `traces_fts` index hasn't refreshed yet (use `--grep`); the term tokenizes
differently than you expect (e.g. `notes.txt` indexes as one lexeme, not
`notes`); or you're relying on semantic matching but `search.toml` still has
`semantic = 0.0`.

**Nothing captured at all.** Confirm the plugin is enabled — `claude plugin list`
should show `teramind@teramind`, and `claude plugin details teramind` should list
6 hooks + 1 MCP server. Check `teramind-hook` is on your `PATH` (the hooks call it
by name). Then check `~/.local/share/teramind/inbox/` — files there mean the
daemon was unreachable when a hook fired; they drain on the next daemon start.
Remember the plugin loads at Claude startup, so it only applies to *new* sessions.

## Uninstall

```sh
# inside Claude Code:
/plugin uninstall teramind@teramind

# then, in a shell:
kill -TERM "$(cat ~/.local/share/teramind/teramindd.pid)"   # stop the daemon (teramind stop is unreliable)
teramind reset                                              # wipe captured local data (optional)
```

## Development

```sh
just            # fmt + clippy + test (default recipe)
just build      # cargo build --workspace
just test       # cargo test --workspace
```

Design specs and implementation plans live under
[docs/superpowers/](docs/superpowers/); operational guides under
[docs/runbooks/](docs/runbooks/).
