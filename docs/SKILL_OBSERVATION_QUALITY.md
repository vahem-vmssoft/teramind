# Skill Observation Quality Testing

How to verify that a specific pattern of interactions (X, Y, Z) causes Teramind
to propose a specific skill (S). Covers the full pipeline: detector → observation
→ synthesis → candidate.

---

## How the pipeline works

```
Sessions captured
      │
      ├─ Detector A (tool_chain)   ─┐
      ├─ Detector B (problem_fix)  ─┼─→ skill_observations (freq++)
      └─ Detector C (llm_proposal) ─┘
                                    │
                              freq ≥ threshold
                                    │
                              CodifierWorker calls LLM
                                    │
                                    ▼
                            skill_candidates (status='pending')
                                    │
                              admin approves
                                    │
                                    ▼
                               skills table
```

There are three detectors, and each has a distinct trigger condition:

### Detector A — `tool_chain`

**Triggers when:** the same ordered sequence of tool calls (by type + head verb/
file extension) appears in ≥2 sessions within the look-back window.

The signature is a SHA-256 over the sequence of `(tool_name, head)` pairs, where:
- `Bash` → head verb of the command (`cargo`, `git`, `grep`, …)
- `Edit` / `Write` / `Read` → file extension (`.rs`, `.toml`, `_`)
- other tools → empty string

**Example trigger (3 sessions all running `cargo test` then editing `.rs`):**
```
Session A: Bash(cargo), Edit(.rs)
Session B: Bash(cargo), Edit(.rs)
Session C: Bash(cargo), Edit(.rs)
→ observation kind=tool_chain, frequency=3
```

**Example NON-trigger:**
```
Session A: Bash(cargo), Edit(.rs)
Session B: Bash(cargo), Edit(.toml)   ← different extension → different signature
→ two separate observations, each frequency=1; no candidate until ≥2 sessions match each
```

---

### Detector B — `problem_fix`

**Triggers when:** a turn's `user_prompt` looks like an error report AND that turn
has an associated file diff in the same session.

**Error indicators** (any one is enough):
```
error:          ← Rust compiler
Error:          ← generic
panicked at     ← Rust panic
Traceback       ← Python
FAILED          ← test runner
clippy::         ← clippy lint
cannot find     ← Rust missing item
undefined reference ← linker
```

The signature hashes the **normalized** error (line numbers and identifiers
stripped) + the diff kind (`added_block`, `removed_block`, `signature_change`,
`rename`, `mixed`). So two sessions hitting the same error class → same signature
→ frequency increments.

**Example trigger:**
```
Session A: user_prompt="error: cannot find `tokio` in scope" + diff adds Cargo.toml dep
Session B: user_prompt="error: cannot find `serde` in scope" + diff adds Cargo.toml dep
→ both normalize to "error: cannot find `<id>` in scope" → same signature → freq=2
```

---

### Detector C — `llm_proposal`

**Triggers on every codifier cycle** (not a pattern threshold). The LLM reads
the last 5 ended sessions' wiki pages (or raw turns if no wiki) and decides
whether to propose a skill. This runs regardless of frequency — it's the
"what did the agent keep doing across recent work?" pass.

Because it's LLM-driven, you cannot assert an exact output — only that a
reasonable proposal appears (see §4 below).

---

## Scenario definition format

Define scenarios in TOML. One file per scenario. Place them under
`benches/codifier-eval/scenarios/`.

```toml
# benches/codifier-eval/scenarios/cargo-test-fix.toml

[scenario]
name        = "cargo-test-then-fix"
description = "Agent runs cargo test, reads the error, edits the source"
detector    = "tool_chain"        # which detector this exercises
min_sessions = 2                  # how many sessions needed to trigger

# Each [[session]] block is one Claude Code session.
# Repeat the same pattern across min_sessions sessions to hit the threshold.

[[session]]
cwd = "/proj/myapp"
[[session.turn]]
user_prompt = "Run the tests."
[[session.turn.tool_call]]
name    = "Bash"
command = "cargo test"
output  = "test result: FAILED. 1 failed;"
[[session.turn.tool_call]]
name      = "Edit"
file_path = "src/lib.rs"
diff      = "- fn broken() {}\n+ fn fixed() {}"

[[session]]
cwd = "/proj/myapp"
[[session.turn]]
user_prompt = "The tests are failing again, please fix."
[[session.turn.tool_call]]
name    = "Bash"
command = "cargo test --lib"
output  = "test result: FAILED. 2 failed;"
[[session.turn.tool_call]]
name      = "Edit"
file_path = "src/main.rs"
diff      = "- let x = 0;\n+ let x = 1;"

# What you expect to observe after replaying these sessions.
[expect]
observation_kind      = "tool_chain"
min_frequency         = 2

# Optional: if you want to assert on the LLM-generated candidate too.
# Requires codify.toml to be set up; see §4.
[expect.candidate]
name_contains    = "run-tests"   # substring match, case-insensitive
body_contains    = "cargo test"
```

### Scenario for problem_fix

```toml
# benches/codifier-eval/scenarios/missing-dep-fix.toml

[scenario]
name        = "missing-crate-dep"
description = "User pastes a compiler error about a missing crate; agent adds dep"
detector    = "problem_fix"
min_sessions = 2

[[session]]
cwd = "/proj/myapp"
[[session.turn]]
user_prompt = "error: can't find crate for `anyhow`"
[[session.turn.tool_call]]
name      = "Edit"
file_path = "Cargo.toml"
diff      = "+anyhow = \"1\"\n"

[[session]]
cwd = "/proj/myapp"
[[session.turn]]
user_prompt = "error: can't find crate for `tokio`"
[[session.turn.tool_call]]
name      = "Edit"
file_path = "Cargo.toml"
diff      = "+tokio = { version = \"1\", features = [\"full\"] }\n"

[expect]
observation_kind  = "problem_fix"
min_frequency     = 2
```

---

## Running a scenario

There is no automated harness yet for codifier scenarios (unlike `teramind-search-eval`).
Use the procedure below. It works against the live local daemon so you get real detector
output, not a simulation.

### Step 1 — inject the scenario sessions into the DB

Write a small helper script per scenario (or one generic one). The key calls are:

```rust
// Pseudocode — adapt from crates/teramindd/tests/codifier_worker_e2e.rs

let pool = teramind_db::testing::fresh_pool().await?;   // OR connect to live daemon DB

// Insert a session
let session_id = Uuid::new_v4();
sqlx::query!(
    "INSERT INTO sessions (id, agent_kind, cwd, started_at, ended_at)
     VALUES ($1, 'claude_code', $2, now() - interval '10 min', now())",
    session_id, cwd
).execute(pool.pg()).await?;

// Insert a turn
let turn_id = Uuid::new_v4();
sqlx::query!(
    "INSERT INTO turns (id, session_id, ordinal, user_prompt, started_at)
     VALUES ($1, $2, 0, $3, now() - interval '5 min')",
    turn_id, session_id, user_prompt
).execute(pool.pg()).await?;

// Insert tool calls
sqlx::query!(
    "INSERT INTO tool_calls (id, turn_id, ordinal, name, input, output, started_at, ended_at)
     VALUES ($1, $2, 0, $3, $4, $5, now() - interval '4 min', now() - interval '3 min')",
    Uuid::new_v4(), turn_id, tool_name,
    serde_json::json!({"command": cmd_text}),
    output_text
).execute(pool.pg()).await?;

// Insert a file diff (needed for problem_fix detector)
sqlx::query!(
    "INSERT INTO file_diffs (id, session_id, turn_id, rel_path, unified_diff, attribution, captured_at)
     VALUES ($1, $2, $3, $4, $5, 'agent', now())",
    Uuid::new_v4(), session_id, turn_id, file_path, diff_text
).execute(pool.pg()).await?;
```

The shell equivalent using `psql` (for quick ad-hoc testing):

```sh
PSQL="PGPASSWORD=teramind ~/.theseus/postgresql/16.13.0/bin/psql -h /tmp -p 54817 -U postgres -d teramind"

SID=$(uuidgen)
TID=$(uuidgen)
CWD="/proj/myapp"
PROMPT="Run the tests."

# session
$PSQL -c "INSERT INTO sessions (id, agent_kind, cwd, started_at, ended_at)
           VALUES ('$SID','claude_code','$CWD',now()-'10 min'::interval,now());"

# turn
$PSQL -c "INSERT INTO turns (id, session_id, ordinal, user_prompt, started_at)
           VALUES ('$TID','$SID',0,'$PROMPT',now()-'5 min'::interval);"

# Bash tool call
$PSQL -c "INSERT INTO tool_calls (id, turn_id, ordinal, name, input, output, started_at, ended_at)
           VALUES ('$(uuidgen)','$TID',0,'Bash',
                   '{\"command\":\"cargo test\"}'::jsonb,
                   'test result: FAILED. 1 failed;',
                   now()-'4 min'::interval, now()-'3 min'::interval);"

# Edit tool call
$PSQL -c "INSERT INTO tool_calls (id, turn_id, ordinal, name, input, output, started_at, ended_at)
           VALUES ('$(uuidgen)','$TID',1,'Edit',
                   '{\"file_path\":\"src/lib.rs\"}'::jsonb,'',
                   now()-'2 min'::interval, now()-'1 min'::interval);"

# File diff (needed for problem_fix; optional for tool_chain)
$PSQL -c "INSERT INTO file_diffs (id, session_id, turn_id, rel_path, unified_diff, attribution, captured_at)
           VALUES ('$(uuidgen)','$SID','$TID','src/lib.rs',
                   '- fn broken() {}\n+ fn fixed() {}','agent',now());"
```

Repeat for each `[[session]]` block in the scenario file.

### Step 2 — trigger the detectors

Detectors run automatically on the codifier cycle, but you can force them
without waiting:

**Option A — wait for the next cycle (default: every 30s)**

```sh
teramind skills observations   # poll until new rows appear
```

**Option B — restart with a short cycle** (dev only)

```sh
# Stop daemon, set env var, restart
kill -TERM "$(cat ~/.local/share/teramind/teramindd.pid)"
TERAMIND_CODIFY_POLL_SECS=5 teramind start
```

### Step 3 — verify the observation appeared

```sh
# Check by kind
teramind skills observations --kind=tool_chain

# Or query the DB directly for the exact signature
PSQL="PGPASSWORD=teramind ~/.theseus/postgresql/16.13.0/bin/psql -h /tmp -p 54817 -U postgres -d teramind"
$PSQL -c "SELECT kind, signature, frequency, status, context_blob
           FROM skill_observations
           ORDER BY last_seen_at DESC LIMIT 10;"
```

**Pass condition:** a row with the expected `kind` exists and `frequency ≥ min_frequency`.

### Step 4 — verify the candidate (synthesis step)

The candidate is generated by the LLM after the observation frequency crosses
`min_observation_frequency` in `codify.toml` (default: 2). With `codify.toml`
configured:

```sh
teramind skills list --filter=pending
```

**For programmatic assertion (in a Rust integration test):**

```rust
let pending = SkillCandidateRepo::new(pool.clone()).list_pending(10).await?;
assert!(!pending.is_empty(), "expected at least one pending candidate");
let c = &pending[0];
assert!(
    c.name.to_lowercase().contains("run-tests"),
    "unexpected name: {}", c.name
);
assert!(
    c.body.contains("cargo test"),
    "body should mention cargo test"
);
```

---

## Example scenarios to author first

These are the highest-value scenarios for your internal use — they cover the most
common real-world patterns before you have enough live data to rely on.

### TC-1. Run tests → fix source

```
Pattern: Bash(cargo) → Edit(.rs)   (repeated ≥2 sessions)
Expected observation: tool_chain
Expected skill name (suggestion): "run-tests-fix-source"
```

### TC-2. Format → lint → commit

```
Pattern: Bash(cargo) → Bash(git)   (repeated ≥2 sessions)
  where first Bash head is "cargo fmt" or "cargo clippy"
  and second is "git commit" or "git push"
Expected observation: tool_chain
Expected skill: "format-lint-commit"
```

### TC-3. Read config → edit config → restart service

```
Pattern: Read(.toml) → Edit(.toml) → Bash(systemctl or kill)
Expected observation: tool_chain
Expected skill: "edit-config-restart"
```

### PF-1. Rust "cannot find" error → add Cargo.toml dep

```
Pattern: user_prompt matches "error: can't find crate for `<id>`"
         + diff on Cargo.toml (added_block)
Expected observation: problem_fix
Expected skill: "add-missing-crate-dependency"
```

### PF-2. Test FAILED → fix source

```
Pattern: user_prompt matches "FAILED: N tests"
         + diff on src/**/*.rs (mixed or signature_change)
Expected observation: problem_fix
Expected skill: "fix-failing-tests"
```

### PF-3. Python Traceback → fix source

```
Pattern: user_prompt matches "Traceback (most recent call last)"
         + any diff
Expected observation: problem_fix
Expected skill: "fix-python-traceback"
```

---

## Detector trigger reference

Quick lookup for writing new scenarios:

| You want to trigger | Detector | Minimum required |
|---|---|---|
| Same tool sequence in N sessions | `tool_chain` | ≥2 sessions with identical `(tool, head)` sequence |
| Error message + code fix | `problem_fix` | ≥2 sessions where `user_prompt` matches error pattern AND turn has a `file_diffs` row |
| Any recurring LLM-level pattern | `llm_proposal` | 1+ ended sessions with `ended_at` set; LLM decides |

Tool head values that matter for `tool_chain` signatures:

| Tool call | What makes the signature |
|---|---|
| `Bash("cargo test")` | `Bash(cargo)` |
| `Bash("git commit -m ...")` | `Bash(git)` |
| `Bash("./run.sh")` | `Bash(run.sh)` |
| `Edit("src/main.rs")` | `Edit(.rs)` |
| `Edit("Cargo.toml")` | `Edit(.toml)` |
| `Edit("Makefile")` | `Edit(_)` |
| `Read("config.yaml")` | `Read(.yaml)` |
| `WebFetch`, `WebSearch`, etc. | `<tool>()` — empty head |

Two sessions only match if the full ordered sequence of these `(tool, head)` tuples
is byte-for-byte identical.

---

## Known gaps to be aware of

- **`--min-freq` filter is broken** (SMOKE_TEST.md B1): `teramind skills observations --min-freq=N` currently ignores the filter and returns all observations. Verify frequency in the DB directly until this is fixed.
- **Codifier is disabled by default**: it requires `~/.config/teramind/codify.toml` to exist. Without it, detectors do not run and no candidates are generated. The synthesis (LLM → candidate) step is not reachable.
- **`llm_proposal` relies on wiki_pages**: if the summarizer is producing empty content (SMOKE_TEST.md B3), the LLM bundler falls back to raw turns, which are much lower signal. Fix the summarizer before evaluating `llm_proposal` quality.
- **No automated harness yet**: unlike `teramind-search-eval` (which has a full corpus generator + metrics runner), there is no equivalent `teramind-codify-eval` binary. The manual procedure above is the current path. If you find yourself re-running these scenarios regularly, that binary is the right investment.
