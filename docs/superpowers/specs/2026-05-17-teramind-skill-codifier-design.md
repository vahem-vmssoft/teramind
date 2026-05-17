# Teramind Skill Codifier — Design Spec

- **Status:** Approved (brainstorming complete; pending implementation plan)
- **Author:** Vahe Momjyan
- **Date:** 2026-05-17
- **Scope:** First post-v1.0 spec on the Teramind roadmap. Adds the `codified` skill source path that has been reserved in the schema since Plan A but never implemented. Mines repeated patterns out of captured sessions into reviewable, scoped skill candidates that — once approved — surface to Claude at SessionStart.

---

## 1. Background and motivation

Plan A reserved three skill sources in the `skills` table: `authored` (created by the agent via `mcp__teramind__save_skill`), `imported` (placeholder for v1.x), and `codified` (mined from session traffic by an automated pipeline). The codified path stayed dormant through all of v1.0. It now matters: with team mode (Plans I–L) shipped, a developer's traces aren't just their own — they're feedstock that the whole team benefits from. The substrate captures every prompt, every tool call, every diff. The pattern is there in the data. Nobody has time to read transcripts and write skill files by hand.

The recurring scenario is concrete. Across a quarter, the same developer runs the same Rust `cargo build && cargo test && git commit` chain on every PR. A teammate ports OpenSSL to OpenVMS x86 and writes the same `AC_CHECK_FUNC([fork]) → vfork-aware probe` patch in three different `configure.ac` files across three different repos. The model in each session has to re-derive the recipe from scratch; the team's accumulated knowledge of *its own working style* lives in 800-MB of session transcripts that nobody opens.

The codifier closes that loop. A cheap detector layer (pure-Rust pattern matching) scans recent traces. Above a threshold (default ≥3 distinct sessions exhibiting the same signature), an LLM synthesis layer decides if the pattern is worth writing up and produces a Markdown skill body with a name, a description, and a scope. The result is a *candidate* — never live yet. An admin SQL `UPDATE` flips status to `approved`, the next worker tick promotes it into the live `skills` table, and the next SessionStart in a matching project surfaces the skill in the auto-recall digest. The agent reads its own institutional knowledge mid-conversation.

---

## 2. Goals and non-goals

### 2.1 In scope (v1 of this spec)

- A two-stage mining pipeline: stage-one detectors → stage-two LLM synthesis.
- Three detector kinds: `tool_chain` (repeated tool-call sequences), `problem_fix` (repeated error → fix shapes), `llm_proposal` (LLM-driven catch-all for patterns rules miss).
- Two entry points feeding the same pipeline: an autonomous `codifier_worker` (timer-driven; default 6 h cycles) and an agent-initiated `mcp__teramind__codify` MCP tool.
- New storage: `skill_observations` (detector output) + `skill_candidates` (post-synthesis, pre-approval). Additive `applies_to_cwds text[]` column on existing `skills`.
- A new `CodifyProvider` trait + Ollama / Anthropic / Null impls, configured at `~/.config/teramind/codify.toml`.
- CLI surfaces (read-only): `teramind skills list [--pending|--rejected]`, `teramind skills show <name|id>`, `teramind skills observations [--kind X] [--min-freq N]`.
- SessionStart auto-recall digest extension: a "Relevant codified skills" section showing up to K (default 5) skills whose `applies_to_cwds` overlap with the current cwd.
- Admin approval: SQL one-liner against `skill_candidates`; promotion happens automatically inside the daemon/server's next worker tick.
- Privacy: a session that has `share = false` (Plan J's `DecisionCache`) never enters any detector's seed set.
- Team mode: codifier_worker runs server-side; local daemons in team mode don't run their own. The MCP tool routes via the existing `RpcTransport`.
- `teramind doctor` gains codifier health lines.

### 2.2 Explicit non-goals (deferred)

- Interactive review (`teramind skills review`, `mcp__teramind__list_candidates` + `approve_candidate`). v1.1.
- Filesystem materialization to `~/.claude/skills/<name>/SKILL.md`. v1.1.
- Web UI for approval. After the general web-UI work.
- Detector weighting / feedback loop from approve/reject signals. v1.2.
- Automated promotion (skipping the admin gate for high-confidence candidates). v2+.
- Skill versioning + rollback. v2+.
- Cross-skill ranking signals (preferring skills the agent actually retrieves). v1.2.
- Hard delete of skills tied to a removed user. Tied to the broader v1.1 hard-delete story.

### 2.3 Success criteria

1. A developer running 5 sessions with the same Rust PR-prep tool-chain, then running a 6th session in a similar Rust project, sees a `rust-pr-prep` candidate appear in `teramind skills list --pending` after the next autonomous cycle (≤ 6 h, often within minutes if MCP-triggered).
2. After SQL-approving the candidate, the developer's *next* SessionStart in the same project surfaces the skill in the auto-recall digest within one digest call.
3. In team mode, a candidate seeded only from sessions a different developer never saw still appears in their digest once approved — shared institutional knowledge is the point.
4. A session in a project the candidate does NOT scope to (e.g., a Python repo when the skill applies to `/openvms-*`) does NOT see the skill in its digest.
5. `cargo test --workspace` adds ~25 new tests across L1/L2/L3 and is green.

---

## 3. Architecture overview

```
                                  ┌─ codifier_worker ──────────────────────┐
                                  │                                         │
sessions → turns → tool_calls →   │ detector_loop (every autonomous_cycle):  │
file_diffs → wiki_pages           │   tool_chain_detector ──┐                │
   │                              │   problem_fix_detector ─┤                │
   │                              │   llm_proposal_detector ┘                │
   │                              │      │                                   │
   ▼                              │      ▼                                   │
SearchRepo  ◄─────────────────────│  UPSERT skill_observations               │
   ▲                              │      │                                   │
   │                              │      ▼ frequency ≥ min                   │
   │                              │  synthesis_loop (every poll):            │
   │                              │   bundle context (redacted)              │
   │                              │   call CodifyProvider                    │
   │                              │      │                                   │
   │                              │      ▼                                   │
   │                              │  INSERT skill_candidates (pending)       │
   │                              │                                          │
   │                              │  promote_loop (every poll):              │
   │                              │   SELECT candidates WHERE status='approved'│
   │                              │   transaction:                           │
   │                              │     INSERT skills(source='codified')     │
   │                              │     UPDATE candidates SET status='promoted'│
   │                              └──────────────────────────────────────────┘
   │
   │   (agent-initiated)
   │   mcp__teramind__codify(seed_session_ids, hint)
   │      → RpcTransport → dispatch(Request::CodifyNow)
   │      → synthetic skill_observations row (kind='llm_proposal')
   │      → worker picks it up next tick
   │
   ▼
do_auto_recall (SessionStart hook):
   …existing recent-sessions + wiki digest…
   + "Relevant codified skills" section (skills filtered by cwd-ancestry)
   → injected into Claude's context
```

### 3.1 Layer responsibilities

| Layer | Responsibility |
|---|---|
| Detectors (3 modules) | Cheap pure-Rust scans of recent traces. Emit typed `Observation` rows. Always running on the autonomous timer. |
| `skill_observations` table | Dedup buffer keyed by `(kind, signature)`. Tracks how many sessions exhibit each pattern and which ones. Survives daemon restarts. |
| `codifier_worker` synthesis loop | One synthesis per poll tick. Pulls oldest `open` observation above threshold. Bundles context, redacts, calls `CodifyProvider`. Writes a `skill_candidate` row or marks observation `skipped`. |
| `CodifyProvider` trait + impls | LLM call. Ollama default; Anthropic gated by `network_egress = true`. Returns either a structured `Skill { name, description, body, applies_to_cwds }` or `Skip { reason }`. |
| `skill_candidates` table | Pre-approval staging. Status: `pending → approved → promoted` (or `rejected` / `superseded`). |
| `codifier_worker` promote loop | Polls for `approved` candidates; promotes each into `skills` with `source='codified'`. Idempotent under daemon crashes. |
| MCP `mcp__teramind__codify` | Agent-initiated entry. Inserts a synthetic `llm_proposal` observation with caller-supplied hint; returns immediately. |
| CLI `teramind skills …` | Read-only views: list, show, observations. No approve / reject subcommands in v1. |
| Hook auto-recall digest | Plan F's `do_auto_recall` gains a "Relevant codified skills" section gated by cwd-ancestry overlap. |

### 3.2 New modules in the workspace

```
crates/teramindd/src/services/codify/
├── mod.rs                 # factory + registry
├── detectors/
│   ├── mod.rs
│   ├── tool_chain.rs      # detector A
│   ├── problem_fix.rs     # detector B
│   ├── llm_proposal.rs    # detector C
│   └── heuristics.rs      # shared error-signature normalizer + chain signatures
├── synthesis.rs           # bundle_context + call provider + parse decision
├── promote.rs             # transactional candidate → skill promotion
├── prompts.rs             # SYSTEM_PROMPT + snapshot tests
├── ollama.rs              # OllamaCodifyProvider
├── anthropic.rs           # AnthropicCodifyProvider (gated)
└── null.rs                # NullCodifyProvider (testing + opt-out)

crates/teramindd/src/services/codifier_worker.rs  # spawn + 3 loops
crates/teramind-db/src/repos/skill_observation.rs  # SkillObservationRepo
crates/teramind-db/src/repos/skill_candidate.rs    # SkillCandidateRepo
crates/teramind-db/migrations/20260518000001_skill_codifier.sql

crates/teramind/src/commands/skills.rs             # list / show / observations
crates/teramind-mcp tool: mcp__teramind__codify    # MCP tool body
```

### 3.3 Reuse

- `SummaryProvider` factory pattern (Plan H) — same shape, different trait name.
- `Redactor::apply` (Plan A) — runs over `bundled_context` before any network egress.
- `do_auto_recall` (Plan F) — extended with one new section assembly call.
- `RpcTransport` (Plan K) — MCP tool and CLI calls both route through it. In team mode the codifier work runs server-side; the local CLI/MCP just talks to it.
- `Request` / `Response` enums (Plan A's IPC contract) — gain `CodifyNow`, `SkillsList`, `SkillsShow`, `SkillsObservations` variants.
- `DecisionCache` (Plan J) — detectors filter out sessions whose decision is `DeniedKeepLocal`.

---

## 4. Storage

### 4.1 Migration `20260518000001_skill_codifier.sql`

```sql
-- Observations: detector output, dedup buffer.
CREATE TABLE skill_observations (
  id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  kind           text NOT NULL CHECK (kind IN ('tool_chain','problem_fix','llm_proposal')),
  signature      text NOT NULL,
  session_ids    uuid[] NOT NULL,
  frequency      integer NOT NULL,
  context_blob   jsonb NOT NULL,
  first_seen_at  timestamptz NOT NULL DEFAULT now(),
  last_seen_at   timestamptz NOT NULL DEFAULT now(),
  status         text NOT NULL DEFAULT 'open'
                   CHECK (status IN ('open','synthesized','skipped'))
);
CREATE UNIQUE INDEX skill_observations_sig ON skill_observations (kind, signature);
CREATE INDEX skill_observations_open_recent
  ON skill_observations (last_seen_at DESC) WHERE status = 'open';

-- Candidates: post-synthesis, pre-approval staging.
CREATE TABLE skill_candidates (
  id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  observation_id      uuid NOT NULL REFERENCES skill_observations(id) ON DELETE CASCADE,
  name                text NOT NULL,
  description         text NOT NULL,
  body                text NOT NULL,
  applies_to_cwds     text[] NOT NULL,
  source_session_ids  uuid[] NOT NULL,
  model               text NOT NULL,
  input_tokens        integer NOT NULL,
  output_tokens       integer NOT NULL,
  generated_at        timestamptz NOT NULL DEFAULT now(),
  status              text NOT NULL DEFAULT 'pending'
                        CHECK (status IN ('pending','approved','rejected','promoted','superseded')),
  reviewer            text,
  reviewed_at         timestamptz
);
CREATE INDEX skill_candidates_pending
  ON skill_candidates (generated_at DESC) WHERE status = 'pending';
CREATE INDEX skill_candidates_obs ON skill_candidates (observation_id);
CREATE UNIQUE INDEX skill_candidates_open_name
  ON skill_candidates (name) WHERE status = 'pending';

-- Skills gain a cwd-scope column.
ALTER TABLE skills ADD COLUMN applies_to_cwds text[] NOT NULL DEFAULT '{}';
CREATE INDEX skills_codified ON skills (updated_at DESC) WHERE source = 'codified';
```

### 4.2 Status lifecycles

**Observation:**

```
open ──── synthesis loop picks
          ├── provider returns Skill   → synthesized   (candidate inserted)
          └── provider returns Skip    → skipped       (no candidate)
```

**Candidate:**

```
pending  ──── admin SQL UPDATE
              ├── status='approved' → promote loop runs → promoted (skill row inserted)
              └── status='rejected' → stays for audit; never surfaced

(or)
pending  ──── newer candidate for same observation_id → superseded
```

### 4.3 Storage estimate

For a busy team (20 engineers, ~30 sessions/day, ~5 years): observations are bounded by *distinct* signatures, not session count. Realistic ceiling: ~10 000 observations / year × 5 years = ~50 k rows. Candidates: ~4 / day × 365 × 5 = ~7 300 rows. Both far smaller than the trace tables. Negligible vs the substrate's existing footprint.

---

## 5. Detectors

### 5.1 Detector A — `tool_chain_detector`

**Input:** turns from the last 30 days (`turns.started_at >= now() - interval '30 days'`).

**Per-session signature:**

1. Gather all `tool_calls` for the session, ordered by `started_at`.
2. Build an ordered list of `(tool_name, head_verb)` tuples. `head_verb` is detector-specific:
   - For `Bash`: first whitespace-tokenized word of the command, lowercased.
   - For `Edit` / `Write` / `Read`: extension of the target file (`.rs`, `.py`, `.toml`, …), or `_` if no extension.
   - For other tools: an empty string.
3. Hash the tuple list with SHA-256; first 16 hex chars form the signature.

**Emit:** group sessions by signature, count distinct sessions. If `count >= min_observation_frequency` and signature isn't already at status `skipped` or `synthesized`, UPSERT a row into `skill_observations` (kind=`tool_chain`, `context_blob = { "head_chain": [...], "modal_cwd": "..." }`).

### 5.2 Detector B — `problem_fix_detector`

**Input:** turn pairs `(t_n, t_{n+1})` where `t_n` is a user prompt and `t_{n+1}` is the assistant turn.

**Filter:** `t_n` matches one of the error patterns in `heuristics.rs::ERROR_PATTERNS` (regex list: `error:`, `panicked at`, `Traceback`, `failed:`, `cargo test FAILED`, `clippy::\w+` for Rust lint names, etc.) AND `t_{n+1}` shipped at least one `file_diffs` row.

**Signature:** `(normalized_error, diff_kind)`, where:
- `normalized_error` strips line/column numbers, replaces `[a-zA-Z_][a-zA-Z0-9_]*` identifiers with `<id>` (capped at 80 chars).
- `diff_kind` ∈ `{added_block, removed_block, signature_change, rename, mixed}` based on a quick AST-free heuristic over the diff text.

**Emit:** group, count distinct sessions, UPSERT same as Detector A (kind=`problem_fix`, `context_blob = { "error": "...", "diff_kind": "..." }`).

### 5.3 Detector C — `llm_proposal_detector`

**Input:** the 5 newest sessions with `ended_at IS NOT NULL` AND effective `share != DeniedKeepLocal`.

**Behavior:** bundles the 5 sessions' wiki summaries (Plan H) into a compact prompt: *"Across these 5 sessions, what reusable skill — if any — would you mine? Return JSON: either `{decision:'none'}` or `{decision:'propose', name:'…', hint:'…'}`."*

**Output:** if `decision = propose`, insert a synthetic observation with `kind = llm_proposal`, `signature = sha256(name)` (so re-proposing the same name dedups), `frequency = 5`, `context_blob = { "hint": "...", "model": "..." }`.

**Rate limit:** at most one LLM proposal call per autonomous cycle. Skipped on cycles that hit `max_pending_candidates`.

### 5.4 Privacy filter

Every detector runs its session candidate set through:

```rust
fn filter_shareable(sessions: &[SessionId], cache: &DecisionCache) -> Vec<SessionId> {
    sessions.iter()
        .filter(|sid| {
            // Treat unknown sessions (cache miss) as shareable in local-first;
            // in team mode, the server-side codifier sees only the sessions
            // that already landed via /v1/ingest, which by construction were
            // share=Allowed at forward time.
            !matches!(cache.get(**sid), Some(ShareDecision::DeniedKeepLocal))
        })
        .copied().collect()
}
```

In team mode this is a no-op (denied sessions never landed). In local-first it actively excludes denied sessions from detection.

---

## 6. Synthesis

### 6.1 Bundling

The worker fetches the observation row, joins to its session ids, and pulls a context bundle:

```rust
pub struct SynthesisContext {
    pub kind: String,
    pub signature: String,
    pub frequency: u32,
    pub seed_sessions: Vec<SessionSummary>,  // up to 5
    pub cwds: Vec<String>,                   // distinct cwds of those sessions
}

pub struct SessionSummary {
    pub session_id: SessionId,
    pub cwd: String,
    pub wiki_excerpt: String,    // first ~3000 chars of wiki_pages.content if any
    pub turn_count: i32,
    pub representative_turns: Vec<RepresentativeTurn>,  // up to 3 per session
    pub representative_diffs: Vec<RepresentativeDiff>,  // up to 2 per session
}
```

The bundler caps total output at `input_char_budget` (default 24 000). When over budget it drops representative_diffs first, then representative_turns, then wiki_excerpt — keeping the shortest signal-rich shape.

Every text field passes through `Redactor::apply` before the bundle is serialized to the LLM prompt.

### 6.2 `CodifyProvider` trait

```rust
#[async_trait]
pub trait CodifyProvider: Send + Sync {
    async fn codify(&self, req: CodifyRequest) -> CodifyResult;
    fn name(&self) -> &str;
}

pub struct CodifyRequest {
    pub observation_kind: String,
    pub bundled_context: String,
    pub frequency: u32,
    pub cwds: Vec<String>,
    pub max_output_tokens: u32,
}

pub struct CodifyResult {
    pub decision: CodifyDecision,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

pub enum CodifyDecision {
    Skip { reason: String },
    Skill {
        name: String,
        description: String,
        body: String,
        applies_to_cwds: Vec<String>,
    },
}
```

### 6.3 Prompt

`crates/teramindd/src/services/codify/prompts.rs` exports a single `SYSTEM_PROMPT: &str`. Behavior is snapshot-tested. Shape:

> You are a skill codifier. Given a repeated pattern observed across multiple AI-coding sessions, decide whether it's worth turning into a reusable skill that a future session could read at SessionStart.
>
> A skill is worth codifying when:
> - The pattern recurs deliberately (not coincidentally).
> - The recipe is *transferable* — it would apply to other sessions in similar projects.
> - Writing it down saves the next session at least a few turns of re-derivation.
>
> Reject patterns that are:
> - Trivial (one tool call, no decision).
> - Project-specific in a way that doesn't generalize (e.g. modifying `MY_PROJECT_X_VERSION` constant).
> - Already well-known (basic git, cargo, npm).
>
> Output strict JSON:
> ```
> {"decision":"skip","reason":"..."}
>   OR
> {"decision":"skill","name":"kebab-case","description":"one line","body":"# Markdown ...","applies_to_cwds":["/path/prefix", "..."]}
> ```
>
> `name` ≤ 60 chars, kebab-case, no spaces.
> `description` ≤ 200 chars, one line.
> `body` ≥ 200 chars, ≤ 4000 chars, valid Markdown. Open with a frontmatter block:
> ```
> ---
> source: codified
> seeded_from: <N> sessions
> first_observed: <YYYY-MM-DD>
> applies_to: <cwd-pattern>
> ---
> ```
> `applies_to_cwds` is a list of absolute path prefixes or globs (`*` allowed in segments). Empty list ⇒ global.

### 6.4 Decision handling

- `Skip` → mark observation `status='skipped'`, store reason in `context_blob.skip_reason`.
- `Skill` → INSERT `skill_candidates` row. Mark observation `status='synthesized'`.
- Provider error (timeout, JSON parse fail, network) → leave observation `open` with `context_blob.last_error = "..."` and `last_attempt_at = now()`. One retry per autonomous cycle; after 3 failed attempts, transition to `skipped` with reason `"synthesis_failed:<error>"`.

### 6.5 Promotion

The promote loop polls every `poll_interval_secs`:

```sql
SELECT id, name, description, body, applies_to_cwds, source_session_ids
FROM skill_candidates
WHERE status = 'approved'
ORDER BY reviewed_at
FOR UPDATE SKIP LOCKED
LIMIT 10;
```

For each row, a transaction:

```sql
BEGIN;
INSERT INTO skills (name, description, body, source, source_session_ids, applies_to_cwds)
VALUES ($1, $2, $3, 'codified', $4, $5)
ON CONFLICT (name) DO UPDATE
  SET description = EXCLUDED.description,
      body        = EXCLUDED.body,
      source_session_ids = EXCLUDED.source_session_ids,
      applies_to_cwds = EXCLUDED.applies_to_cwds,
      updated_at = now()
RETURNING id;

UPDATE skill_candidates SET status = 'promoted' WHERE id = $candidate_id;
COMMIT;
```

`ON CONFLICT` handles the case where an `authored` skill with the same name already exists: it gets overwritten with the codified body (the admin approved the overwrite by approving the candidate). The audit row in `skill_candidates` records the swap.

---

## 7. MCP tool + CLI surfaces

### 7.1 `mcp__teramind__codify`

```rust
#[derive(Deserialize, JsonSchema)]
pub struct CodifyArgs {
    /// Sessions to seed the proposal. Empty = let the daemon pick recent.
    #[serde(default)]
    pub seed_session_ids: Vec<String>,
    /// Optional one-line hint about what pattern to look for.
    pub hint: Option<String>,
}
```

Dispatch: `Request::CodifyNow { seed_session_ids, hint }` → server's `dispatch` inserts a synthetic `skill_observations` row with `kind = 'llm_proposal'`, `signature = sha256(hint.unwrap_or(""))`, `session_ids = seed_session_ids`, `frequency = max(seed_session_ids.len(), min_observation_frequency)`. Returns `{"queued": true, "observation_id": "..."}` to the agent. The synthesis happens on the next worker tick.

### 7.2 CLI commands

```
teramind skills list                       # all skills, table form
teramind skills list --pending             # candidates with status=pending
teramind skills list --rejected            # audit view
teramind skills show <name|id>             # full body
teramind skills observations               # recent observations, table form
teramind skills observations --kind tool_chain --min-freq 2
teramind skills observations --status synthesized
```

All commands route through the existing IPC / RpcTransport (Plan K). Server-side dispatch arms: `Request::SkillsList { filter }`, `Request::SkillsShow { name }`, `Request::SkillsObservations { kind, min_freq, status }`.

No `approve`/`reject` subcommands in v1. The intentional v1 admin path is SQL:

```sql
UPDATE skill_candidates
SET status = 'approved', reviewer = 'alice@acme.dev', reviewed_at = now()
WHERE id = '...';
```

Promotion happens automatically in the next poll cycle (≤ 30s by default).

---

## 8. Auto-load via SessionStart digest

### 8.1 Extension to `do_auto_recall`

Plan F's `do_auto_recall` returns a Markdown digest. We add one new section, ordered *before* the recent-sessions block:

```rust
async fn relevant_codified_skills(
    pool: &DbPool,
    cwd: &Path,
    top_k: usize,
) -> Vec<RelevantSkill> {
    // SELECT id, name, description, applies_to_cwds, array_length(source_session_ids, 1)
    // FROM skills
    // WHERE source = 'codified'
    //   AND (applies_to_cwds = '{}' OR EXISTS (
    //         SELECT 1 FROM unnest(applies_to_cwds) AS pattern
    //         WHERE matches_glob(pattern, cwd)
    //       ))
    // ORDER BY updated_at DESC
    // LIMIT $top_k;
}
```

`matches_glob` is a pure-Rust helper in `teramindd::services::codify::glob` — prefix match with `*` segment wildcards. We do not pull in a full glob crate; the pattern language is intentionally tiny.

### 8.2 Digest format

If at least one match, append:

```markdown
## Relevant codified skills

- **rust-pr-prep** — Build + test + commit dance for Rust crates. _(seeded from 4 sessions)_
- **vms-autoconf-fork-probe** — Replace `AC_CHECK_FUNC([fork])` with a vfork-aware probe on OpenVMS x86. _(seeded from 3 sessions)_

To recall the full body of any skill: `mcp__teramind__search` with the skill name.
```

If zero matches, the section is omitted entirely (no empty header).

### 8.3 Body retrieval

A codified skill's full body is retrievable via the existing `mcp__teramind__search` tool — `skills.body` participates in `traces_fts` via Plan A's UNION. The digest deliberately surfaces only name + description + provenance to keep the SessionStart context overhead bounded (~3 lines per skill).

### 8.4 Provenance in the body

Every codified skill's body opens with a frontmatter block:

```markdown
---
source: codified
seeded_from: 4 sessions
first_observed: 2026-05-10
applies_to: /openvms-*
---

# rust-pr-prep
...
```

The LLM emits this frontmatter as part of `CodifyResult::Skill.body`. When the agent retrieves the body it sees provenance immediately — this lets it weigh codified vs authored vs imported skills if multiple have similar names.

### 8.5 Local-first vs team mode

Identical path. In team mode, the hook's `RpcTransport` is `HttpsTransport`, so `do_auto_recall` runs against the server's `skills` table. Privacy note: a skill seeded from sessions a developer never had visibility into still appears in their digest once approved. Knowledge sharing — gated only at the admin approval step — is the point of the substrate.

---

## 9. Configuration

### 9.1 `~/.config/teramind/codify.toml`

```toml
provider = "ollama"                # ollama | anthropic | null
model    = "qwen3.6:latest"

# LLM budgets per synthesize call
input_char_budget    = 24000
output_token_budget  = 1500

# Cadence
poll_interval_secs        = 30
autonomous_cycle_secs     = 21600        # 6h
min_observation_frequency = 3
max_pending_candidates    = 50
digest_top_k              = 5

[detectors]
tool_chain   = true
problem_fix  = true
llm_proposal = true
```

### 9.2 Cloud-provider gating

Identical pattern to Plans G and H. Anthropic codify provider refuses to construct unless `~/.config/teramind/secrets.toml` has `network_egress = true` and `anthropic_api_key = "..."`. No separate secrets file.

### 9.3 Team mode

Server-side config at `/etc/teramind-sync-server/codify.toml` (operator-owned). Local daemons in team mode don't run their own codifier — `teramind doctor` shows `codifier: routed to server (https://...)`. The server's `teramind-sync-server doctor` (a follow-on; not in this spec) would surface the server-side stats.

### 9.4 `teramind doctor` extension

New lines whenever the codifier is configured:

```
codifier:    enabled (ollama: qwen3.6:latest)
observations: 47 open (12 above threshold), 312 synthesized, 28 skipped
candidates:  3 pending review, 11 promoted, 2 rejected
last run:    synthesis 4m ago, detectors 1h ago
```

When team-routed:

```
codifier:    routed to server (https://teramind.acme.dev)
```

When disabled:

```
codifier:    disabled (no codify.toml)
```

---

## 10. Testing strategy

### 10.1 L1 — Unit (pure logic)

- `tool_chain_detector::signature` deterministic; reordering of irrelevant arg fields doesn't change the hash.
- `problem_fix_detector::error_signature` normalizes line numbers, paths, and identifiers; proptest: inputs differing only in those axes hash identically.
- `applies_to_cwds` glob match: prefix, segment-wildcards, ancestor inclusion, root case.
- `CodifyDecision` JSON round-trips both variants; rejects malformed payloads with a typed error.
- Observation status state machine: `open → synthesized | skipped`; illegal transitions error.
- Candidate status state machine: `pending → approved → promoted | rejected | superseded`; illegal transitions error.
- Bundler caps total output at `input_char_budget`; degrades by dropping diffs → turns → wiki_excerpt in that order.

### 10.2 L2 — Component (per-crate, real Postgres)

- Migration `20260518000001_skill_codifier.sql` applies cleanly. Both new tables + the new `applies_to_cwds` column exist.
- `SkillObservationRepo::upsert` merges session_ids and bumps frequency on key conflict.
- `SkillCandidateRepo` round-trips: insert → list_pending → mark_approved → promote.
- Detector A end-to-end on a seeded corpus: 5 sessions with identical Bash→Edit→Bash signatures produce exactly one observation with `frequency = 5`.
- Detector B end-to-end: 4 sessions with `cargo test FAILED` + a subsequent diff produce one observation.

### 10.3 L3 — Integration (daemon subprocess + worker)

- Worker promotes an `approved` candidate within one poll cycle; the new `skills` row has `source = 'codified'`.
- Synthesis loop with a mock `CodifyProvider` returning `Skip { reason }`: observation → `skipped`, no candidate.
- Synthesis with a mock provider returning `Skill { ... }`: candidate appears.
- Back-pressure: `max_pending_candidates = 1` with one existing pending → next tick skips synthesis with `back_pressured = true` logged.
- MCP tool `mcp__teramind__codify`: dispatch produces an `llm_proposal` observation with the supplied `seed_session_ids` and `hint` recorded.
- Privacy: a `share = false` session never appears in any detector's seed set.
- Crash recovery: kill the daemon mid-promotion (between INSERT skills and UPDATE candidate). Restart. The next promote-loop tick re-runs the promotion idempotently via `ON CONFLICT (name) DO UPDATE` and the candidate transitions to `promoted` on the second attempt.

### 10.4 L4 — E2E with real Claude Code (nightly)

- Two scripted sessions run the same Rust PR-prep tool-chain. Codifier cycle runs. The 3rd session in a similar Rust project sees `rust-pr-prep` in its SessionStart digest. Claude observed retrieving + using the recipe.

### 10.5 L5 — Search effectiveness with codified skills

- Plan F's L5 corpus generator plants 5 known codified skills + 10 queries that should retrieve them. New baseline `baseline-codified.json`. Gated like the existing two (lexical, semantic): regression >2 pp nDCG@10 fails CI.

### 10.6 Property + fault-injection

- Proptest: any N synthetic sessions produce at most N observations (no detector cycle generates more rows than its input).
- Hallucination guard: a provider returning a `Skill { name }` colliding with an existing authored skill whose `source_session_ids` is disjoint is rejected with `"name collision with existing authored skill"` rather than overwriting.
- Synthesis timeout (90s): on timeout, observation goes back to `open` with one retry; after 3 failed attempts, status transitions to `skipped` with reason `"synthesis_failed"`.

### 10.7 Performance budgets

| Path | Budget |
|---|---|
| Detector A on 1k-session corpus | p99 < 500 ms |
| Detector B on 1k-session corpus | p99 < 1 s |
| Detector C (one LLM call) | bounded by `output_token_budget` |
| Synthesis call | p99 < 90 s; one retry on timeout |
| Promotion transaction | p99 < 50 ms |
| `do_auto_recall` total (with codified-skills section) | p99 < 200 ms (unchanged from Plan F target) |

---

## 11. Rollout, dependencies, risks

### 11.1 Phases

1. **v1 (this spec).** Pipeline, three detectors, MCP + CLI entry points, SessionStart digest extension, SQL-based admin approval.
2. **v1.1.** Interactive review CLI (`teramind skills review`), MCP `mcp__teramind__list_candidates` + `approve_candidate`, filesystem materialization to `~/.claude/skills/<name>/SKILL.md`.
3. **v1.2.** Detector weighting from approve/reject signals (rejected `problem_fix` signatures downweighted on re-emit). Cross-skill ranking signals.
4. **v2+.** Automated promotion for high-confidence candidates. Skill versioning + rollback.

### 11.2 Dependencies

Builds on Plans A, F, H, K, L. Skill-level surfaces don't depend on Plan I/J (the codifier can run local-first day one), but the team-mode story benefits from them being in place.

### 11.3 Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| LLM produces low-quality skill bodies, agent ignores them | Medium | Admin gate. v1.2 weighting loop closes the feedback. |
| Candidate backlog accumulates indefinitely | Medium | `max_pending_candidates` back-pressure (default 50). `teramind doctor` surfaces backlog. |
| Detector A signature collisions across unrelated workflows | Medium | Signature includes head-verb fingerprint; LLM judge in stage 2 rejects nonsensical bundles. |
| Codifier prompts leak secrets despite redaction | Low | Same `Redactor::apply` path with same property-test corpus as Plans A / H. |
| Token spend if Anthropic is the provider | Medium | 6-hour autonomous cycles + threshold = 3 caps to ~4 synthesis calls/day. Anthropic gated by `network_egress = true`. |
| `mcp__teramind__codify` lets the agent burn tokens unboundedly | Low | The MCP tool queues, doesn't synchronously run. Synthesis rate-limited by worker cadence. |
| Codified skills surface in unrelated projects | Low | `applies_to_cwds` scoping + digest filter requires path overlap. |
| Name collisions between authored + codified skills | Low | `ON CONFLICT (name) DO UPDATE` semantics — codified overwrites authored *only* when admin approved the codified candidate. |
| Pre-existing pgvector install race in test infra | Known | Documented in earlier plans. Serial test runs work in isolation. Not blocking. |

### 11.4 Out of scope

Listed in §2.2. The most important deferral is **interactive review** (v1.1) — until then, admins approve via SQL, which is intentional friction during the calibration period when we want each approval to be deliberate.

---

## 12. Glossary

- **Observation** — A row in `skill_observations`. Detector output. Records what was repeated (by signature), in which sessions, how many.
- **Candidate** — A row in `skill_candidates`. Synthesized skill body produced by the LLM in response to an observation above threshold. Pre-approval.
- **Skill** — A row in `skills`. Promoted candidate (or authored / imported). Visible to the agent via `mcp__teramind__search` and the SessionStart digest.
- **Signature** — Deterministic hash of a pattern, used as the dedup key in `skill_observations`. Same signature ⇒ same observation row, growing `frequency` and `session_ids`.
- **`applies_to_cwds`** — A list of path prefixes / globs. Empty = global. Used to filter the SessionStart digest by cwd-ancestry overlap.
- **Promotion** — Transactional INSERT into `skills` (status `'codified'`) + UPDATE of the candidate to `'promoted'`. Runs automatically on the worker's next tick after admin approval.
- **Codifier worker** — The new `codifier_worker` service. Three loops: detector_loop (long cycle), synthesis_loop (poll), promote_loop (poll).
- **`CodifyProvider`** — Trait that abstracts LLM calls for synthesis. Ollama default; Anthropic gated by `network_egress`.

---

*End of spec.*
