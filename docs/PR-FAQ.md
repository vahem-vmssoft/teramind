# Teramind — PR/FAQ

*Internal document. Working backwards from launch. Subject to change before GA.*

---

## Press Release

### **Teramind: Stop reinventing the wheel — turn every AI coding session into searchable team knowledge.**

**Local-first. Zero-config. Works the moment your agent ships its first turn.**

**SAN FRANCISCO — November 4, 2026 —** Today Teramind announces general availability of an open-source knowledge substrate that captures every interaction a developer has with their AI coding agents, makes prior sessions semantically searchable across the team, and writes a Markdown wiki for every session automatically. Teramind installs in 60 seconds, runs entirely on the developer's machine by default, and integrates with Claude Code on the first `teramind claude install`.

For the last two years, AI coding agents have made individual developers extraordinarily productive. They have also created a new and surprisingly expensive problem: **every solved bug, every clever refactor, every hard-won porting trick lives for exactly one session and then evaporates**. A team of six engineers, each running Claude Code daily, regenerates the same answers to the same problems thousands of times per year. The institutional knowledge that used to live in PR threads, Slack channels, and the heads of senior engineers now lives in chat transcripts that nobody reads twice.

Teramind closes that loop. A tiny shim hooks into Claude Code at session start. Every prompt, every assistant response, every tool call, and every file diff streams into a local Postgres database. Search is available three ways: a `teramind search` CLI, slash commands inside Claude Code, and — most importantly — an MCP server that exposes `mcp__teramind__search`, `mcp__teramind__recall`, and `mcp__teramind__wiki` tools directly to the agent. The model itself can pull a previous solution mid-conversation. When a session ends, a background worker generates a Markdown wiki page summarizing what changed, what decisions were made, and what's left to do — searchable alongside the raw trace and surfaced automatically when a new session starts in the same project.

> "I ported OpenSSL to OpenVMS x86 last quarter. Three months later when I started rsync, Claude pulled up the exact `configure.ac` patches I'd written, with the surrounding conversation, and asked if it should adapt them for the new project. That used to be a half-day of digging through git history. Now it's a fifty-millisecond IPC call." — *Sergey K., Staff Engineer, fictional porting team*

Under the hood, Teramind runs a single Rust daemon (`teramindd`) plus three thin client binaries: the CLI, a hook shim, and an MCP server. The daemon manages an embedded Postgres with `pg_trgm`, `pgcrypto`, and `pgvector` extensions. Search is a five-way blend: full-text indexing (tsvector), trigram similarity for code-shaped queries, **semantic search via local embeddings** (Ollama or in-process FastEmbed), recency decay, and same-project boost. The blend is tunable; the L5 effectiveness benchmark with CI regression gates ensures changes don't degrade quality. Filesystem changes are captured per-turn with attribution (agent vs. human edit). Secrets are redacted before any persistence — never leave the developer's machine unless the user explicitly opts into a cloud provider.

> "We built Teramind because the most valuable thing about working with an AI agent isn't the agent's output — it's the *context* the agent accumulates as it works. Throwing that context away every 8 hours is a tax we kept paying because no one had built the substrate to keep it. Teramind is that substrate, and it's local-first by default so it works inside the firewalls of teams who need it most." — *Vahe Momjyan, creator of Teramind*

**Getting started.** On macOS and Linux:

```
curl -fsSL https://get.teramind.dev/install.sh | sh
teramind init
teramind claude install
```

On Windows:

```
irm https://get.teramind.dev/install.ps1 | iex
teramind init
teramind claude install
```

Teramind is open source under Apache-2.0 at `https://github.com/teramind/teramind`. Releases are signed and notarized. The semantic search and session-summarizer features default to local Ollama; the `nomic-embed-text-v2-moe` (embeddings) and `qwen3.6:latest` (summaries) models are pulled on first use.

---

## Customer FAQs

### What problem does Teramind solve?

Coding-agent sessions are episodic: each conversation is isolated from every conversation before it. Engineers re-discover the same patches, re-debug the same edge cases, and re-explain the same project conventions to the agent. On teams, this multiplies — different engineers on different continents independently solve identical problems. Teramind keeps every session and makes prior work searchable + semantically retrievable by the agent itself, mid-session.

### How is this different from saving Claude conversations to a markdown file?

Three differences. First, **structure**: Teramind separates prompts, assistant responses, tool calls, file diffs, and session metadata into queryable tables — you can search "what Edit commands ran on `parser.rs`" or "all sessions in this project that used Bash with cargo test." Second, **semantic recall**: pgvector + a local embedding model means the agent finds *conceptually* relevant prior work even when the words don't match. Third, **agent integration**: an MCP server gives the agent itself tools to query prior context without the user having to remember to do so.

### Does my code or my conversations leave my machine?

No, by default. Embedded Postgres runs locally; embeddings go to Ollama on `localhost:11434`; session summarization goes to Ollama by default. Cloud providers (Anthropic, OpenAI, Voyage) are wired but refuse to construct unless you explicitly set `network_egress = true` in `~/.config/teramind/embed.toml` or `summarize.toml`. A redaction pass strips AWS keys, GitHub PATs, PEM blocks, and other common secrets *before* any LLM call regardless of provider.

### What does Teramind cost?

Free and open source under Apache-2.0. The only running costs are local: ~20 GB of disk at the 10,000-session scale (per the design's sizing model), and the GPU/CPU your local Ollama uses for embeddings + summaries. Cloud providers are opt-in and use your own API key.

### Can my team share knowledge across machines?

Not yet in v1. Teramind's first release is single-user, single-machine — by design, so the local-first promise is concrete. Multi-machine team sync is the next planned spec (follow-on #4 in the roadmap); until it ships, teams who want cross-machine knowledge sharing can point multiple installs at a single shared Postgres (architectural deviation, breaks local-first) or wait for the sync server.

### Which coding agents does Teramind support?

Claude Code in v1. The capture layer is agent-agnostic by design — the schema has an `agent_kind` column and the IPC contract is provider-neutral — so adding Codex, Cursor, Hermes, and Pi connectors is a follow-on spec rather than a rewrite. Connectors for those agents are next on the roadmap after team sync.

### How fast is it?

Capture is non-blocking; the hook never delays Claude by more than ~15 ms p99. Search returns in under 800 ms p99 on a 10,000-session corpus (target; ceiling is 5 s). Semantic queries against local Ollama are typically 50–200 ms. The summarizer runs asynchronously and never blocks anything else; a session ends, a summary appears within ~60 s on consumer hardware.

### What if Ollama isn't installed?

`teramind init` prints an actionable message and falls back to `FastEmbedProvider`, an in-process embedding model (`nomic-embed-text-v1.5`, ~150 MB) bundled with the binary. For summarization, you'll need either Ollama or a cloud provider (Anthropic/OpenAI); without one of those, session-end summaries are skipped but every other surface still works.

### How do I see what's been captured?

`teramind sessions show` prints the most recent session's wiki page for the current directory. `teramind sessions show <session-id>` for a specific session. `teramind search "<query>"` for raw retrieval. `teramind status` for the daemon's queue depths, drop counters, and provider health. `teramind doctor` for a pasteable diagnostic report.

### What if I want to forget a specific session?

`teramind reset --confirm` deletes all local data. `teramind uninstall --purge --confirm` removes data + config + binaries. For surgical deletion, `DELETE FROM sessions WHERE id = '...'` against the local Postgres cascades through turns, tool_calls, file_diffs, and the wiki page.

### Is the L5 benchmark public?

Yes. The corpus + queries + qrels live in the repo at `benches/search-eval/`. The committed baseline (`baseline.json` lexical-only, `baseline-semantic.json` with semantic enabled) is recomputed on `main` and gates PRs that touch search code. Today's lexical baseline: nDCG@10 = 0.140. Today's semantic baseline (with Ollama `nomic-embed-text-v2-moe`): nDCG@10 = 0.537 — a **3.8× quality lift** from adding pgvector.

---

## Internal FAQs

### Why is this single-user in v1?

We made the explicit tradeoff that getting capture, search, and the MCP integration *right* for one developer was a precondition for getting them right across a team. Multi-tenancy, auth, replication, conflict resolution, and privacy boundaries are each a non-trivial subsystem; trying to ship all of them in v1 would have either delayed the substrate by quarters or shipped a thin and buggy team product. The schema, IPC contract, and search service are designed to be team-sync ready (Plan #4 in the follow-on roadmap) without architectural rework.

### Why Rust, why Postgres, why embedded?

**Rust** because the daemon needs to be small, fast, and reliable enough that users forget it's running. **Postgres** because (a) full-text + trigram + pgvector are first-class, (b) a real query engine matters once corpora pass ~1000 sessions, and (c) the operator story for "where is my data" is "it's in a `.local/share/teramind/pgdata/` directory I can `pg_dump`." **Embedded** because the moment we required users to install + configure Postgres, the install-curve broke. `postgresql_embedded` 0.20 + `postgresql_extensions` 0.20 download a per-arch PG bundle and install pgvector at first run. Tested on macOS arm64, macOS x86_64, Linux x86_64, Linux arm64, Windows x86_64 (Windows arm64 builds but is unsigned in v1).

### Why default Ollama instead of Anthropic for embeddings + summaries?

The local-first promise from the Core spec was non-negotiable: a Teramind install must work behind any firewall, in any compliance regime, without phoning home. Ollama runs entirely on `localhost`. The Anthropic-default path was the alternative — easier first-run for users who already have a Claude key — but it breaks the no-outbound-calls promise that's a hard requirement for enterprise adoption. We chose to optimize for the harder-deployment-environment case and make cloud opt-in with a single config flag.

### What's the runaway-cost story for cloud providers?

There isn't a daemon-side cap — by design. We removed the original `max_summary_per_day` knob in spec review because (a) silently dropping summaries is worse than the cost it was guarding against, (b) the natural rate limit ("one summary per ended session") makes the worst case bounded, and (c) every cloud vendor exposes its own per-month spend caps on the dashboard. `teramind doctor` surfaces total tokens-in/tokens-out so users can monitor; if spend becomes a problem, the answer is `provider = "ollama"`.

### What's the security review status?

Plan E (installers + release CI) includes a `cargo clippy --workspace -- -D warnings` gate on every PR, a `cargo audit` job, and macOS notarization via the Apple Developer ID. Releases are checksummed (`SHA256SUMS`) and optionally signed with cosign keyless OIDC. The redaction layer has property tests asserting no secret from a corpus of sample inputs ever survives `Redactor::apply()`. We have not yet had a third-party security audit; this is on the v1.1 roadmap.

### How does the L5 benchmark work and why should I trust the numbers?

`crates/teramind-search-eval/` is a separate bin crate. It generates a deterministic 500-session synthetic corpus from a fixed RNG seed, runs 100 hand-curated queries across 5 intent classes (natural language, stack trace, code snippet, tool-typed, symbolic/path), and computes nDCG@10, MRR, P@5, P@10, R@10 — per class and overall. The baseline JSON is committed to the repo and regenerated on `main`. PRs touching search code run the eval and refuse to merge if metrics drop more than 2 pp overall, 5 pp per class, 0.03 absolute MRR, or 3 s p95 latency. The semantic baseline is gated separately (`baseline-semantic.json`) so a regression in either path is visible.

### What's left after v1.0?

Roadmap, in dependency order:

1. **Team sync server** (the missing piece for multi-continent teams).
2. **Skill codifier** (mines repeated patterns into reusable skill files Claude auto-loads).
3. **Codex / Cursor / Hermes / Pi connectors** (the agent-agnostic schema is already in place).
4. **Web UI / dashboard** (read-only views over the existing schema).
5. **Hosted SaaS offering** (optional managed sync for teams that don't want to self-host).

### How big is the codebase?

8 crates in a Rust workspace, ~25,000 lines of Rust at v1.0. ~150 commits across plans A–H. Test layers L1 through L5 cover ~600 unit tests, ~80 integration tests, and ~10 nightly E2E tests against real Claude Code.

### What does adoption look like?

Designed for grassroots, individual-engineer adoption first. A single developer can install in 60 seconds, see value within the first session (auto-recall surfaces prior project context), and pay zero coordination cost. Team rollout is then bottom-up: once half the team is using Teramind individually, the team-sync server proposal becomes obvious to anyone who's used it for a week.

### Why "Teramind"?

A working title that stuck. Tera = trillions of bits of trace data; mind = the substrate that makes it usable. Open to a rename before GA if anyone has a better idea.

---

## Appendix: What ships in v1.0

| Subsystem | Description | Status |
|---|---|---|
| Plan A — Daemon + IPC + schema + CLI core | Embedded PG, JSON-RPC over UDS/named pipe, sessions/turns/tool_calls/file_diffs/skills tables, basic CLI | Merged |
| Plan B — Claude Code capture | SessionStart/UserPromptSubmit/PreToolUse/PostToolUse/Stop/PreCompact hooks, deterministic IDs, inbox fallback | Merged |
| Plan C — Search + MCP | `teramind search` CLI, `mcp__teramind__search` + `recall` + `save_skill` tools, slash commands, auto-recall digest | Merged |
| Plan D — FS watcher | Per-turn file_diffs with agent/human attribution, snapshot cache, git-index fallback | Merged |
| Plan E — Installers + release CI | `install.sh` / `install.ps1`, 6-target build matrix, SHA256SUMS, cosign signing, macOS notarization | Merged |
| Plan F — L5 search benchmark | 500-session corpus, 100 queries, nDCG@10/MRR/P@K/R@K gates, fail-soft semantic eval mode | Merged |
| Plan G — pgvector semantic search | EmbeddingProvider trait, OllamaProvider + FastEmbedProvider, async embedding_worker, HNSW index, semantic blend term | Merged |
| Plan H — Session summarizer | SummaryProvider trait, OllamaChatProvider + AnthropicProvider, summarizer_worker, wiki_pages table, `mcp__teramind__wiki` tool, `teramind sessions show` CLI, traces_fts UNION includes wiki | Merged |

---

*Document last updated: 2026-05-17. Maintained by the Teramind core team.*
