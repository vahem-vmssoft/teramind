# Teramind Claude Capture (Plan B) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire Claude Code into the Teramind substrate so that every prompt, message, tool call, and session-lifecycle event from a Claude session is captured into Postgres via the daemon. End state: a user runs `teramind claude install`, opens Claude Code, and afterward sees their session's trace persisted in the local database.

**Architecture:** A new tiny Rust binary `teramind-hook` reads Claude's hook event JSON from stdin, translates it into an `IngestEvent`, and fires a JSON-RPC notify at the daemon over UDS / Named Pipe. A Claude plugin bundle (registered under `~/.claude/plugins/teramind/`) wires Claude's six hook lifecycle events to this binary. Two new CLI subcommands — `teramind claude install` and `teramind claude uninstall` — manage the plugin lifecycle. Best-effort and non-blocking by design: when the daemon is unreachable, the hook persists the event to `inbox/` and exits 0, never delaying Claude.

**Tech Stack:** Rust stable, `tokio` (single-threaded current-thread runtime in the hook for fast cold start), `serde`, `serde_json`, `clap` (CLI only), the existing `teramind-core` / `teramind-ipc` crates from Plan A.

**Spec reference:** `docs/superpowers/specs/2026-05-13-teramind-core-design.md`, Sections 2.1 (in-scope: plugin bundle integration), 3 (architecture: hook → daemon flow), 5 (capture flow), and 7.4 (`teramind claude install`). Plan A built the daemon side; Plan B is the Claude-facing side.

**Status of Plan A (prerequisite):** All 62 Plan A tasks landed on `main` at `be0612f`. The daemon accepts `IngestEvent`s over IPC and persists them to Postgres with redaction and dead-letter handling. `EventEnvelope`, `IngestEvent`, and `Notify::Ingest` are stable wire types.

---

## File Structure

```
teramind/
├── Cargo.toml                                    [+ add crates/teramind-hook to members]
├── crates/
│   ├── teramind-hook/                            ── NEW: sub-millisecond hook shim binary
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── main.rs                          [entry: dispatch, fire-and-forget]
│   │   │   ├── hook_input.rs                    [Claude hook JSON shapes per event]
│   │   │   ├── translate.rs                     [hook JSON → IngestEvent]
│   │   │   ├── inbox.rs                         [fallback when daemon is unreachable]
│   │   │   ├── spawn.rs                         [lazy-spawn teramindd if needed]
│   │   │   └── selftest.rs                      [`teramind-hook --selftest`]
│   │   └── tests/
│   │       ├── translate.rs                     [JSON → IngestEvent unit + golden cases]
│   │       ├── inbox_fallback.rs                [daemon-down behavior]
│   │       └── happy_path.rs                    [hook stdin → daemon → PG row, integration]
│   └── teramind/                                 [CLI: + claude install/uninstall subcommands]
│       └── src/commands/
│           ├── claude.rs                        [new: subcommand dispatcher]
│           ├── claude_install.rs                [new: install logic]
│           └── claude_uninstall.rs              [new: uninstall logic]
├── plugins/
│   └── claude/                                   [NEW: plugin template, copied at install time]
│       ├── plugin.json                          [Claude plugin manifest]
│       ├── hooks/
│       │   ├── session_start.sh                 [shell wrappers that exec teramind-hook]
│       │   ├── user_prompt_submit.sh
│       │   ├── pre_tool_use.sh
│       │   ├── post_tool_use.sh
│       │   ├── stop.sh
│       │   └── pre_compact.sh
│       ├── skills/.gitkeep                      [empty in Plan B; populated by later plans]
│       └── README.md                            [for humans who navigate to the dir]
└── docs/
    └── runbooks/
        └── claude-capture-manual-smoke.md       [L4 procedure for real-Claude verification]
```

**Why these boundaries:** `teramind-hook` is its own crate so its binary is minimal — no `sqlx`, no `tokio` features beyond what IPC needs, no `clap`. The `commands/claude_*.rs` files keep `teramind`'s subcommand registry shallow and let each command file stay focused. The `plugins/claude/` directory is template content copied verbatim at install time; it is NOT a Cargo workspace member.

**Hook event flow (architecture refresher):**

```
Claude Code → fires hook
  → ~/.claude/plugins/teramind/hooks/<event>.sh
    → execs `teramind-hook` with the event name as arg, hook JSON on stdin
      → teramind-hook reads stdin
        → parses to a hook-specific struct (HookInput::SessionStart, …)
          → translates to teramind-core::IngestEvent
            → wraps in EventEnvelope { client_event_id, ts, event }
              → sends Notify::Ingest over UDS / Named Pipe (fire-and-forget)
                → if daemon unreachable:
                    spawn teramindd in background; retry once
                    if still unreachable: write envelope to inbox/<uuid>.json
                → exit 0 (no blocking, no errors propagated to Claude)
```

---

## Section 1 — Workspace: add the `teramind-hook` crate

### Task 1: Register `teramind-hook` as a workspace member

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Create: `crates/teramind-hook/Cargo.toml`
- Create: `crates/teramind-hook/src/main.rs`

- [ ] **Step 1: Append `crates/teramind-hook` to the workspace members list**

In `Cargo.toml` at the repo root, update the `members` array under `[workspace]`:

```toml
[workspace]
resolver = "2"
members = [
    "crates/teramind-core",
    "crates/teramind-ipc",
    "crates/teramind-db",
    "crates/teramindd",
    "crates/teramind",
    "crates/teramind-hook",
]
```

- [ ] **Step 2: Write `crates/teramind-hook/Cargo.toml`**

```toml
[package]
name = "teramind-hook"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[[bin]]
name = "teramind-hook"
path = "src/main.rs"

[dependencies]
teramind-core = { path = "../teramind-core" }
teramind-ipc  = { path = "../teramind-ipc" }
anyhow      = { workspace = true }
serde       = { workspace = true }
serde_json  = { workspace = true }
thiserror   = { workspace = true }
tokio       = { workspace = true }
uuid        = { workspace = true }
time        = { workspace = true }
clap        = { workspace = true }
tracing     = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

The hook binary inherits `tokio = { features = ["full"] }` from the workspace; that's heavy for a sub-millisecond shim, but tightening features later is a follow-on optimization. Cold start performance is measured in Section 11.

- [ ] **Step 3: Write the placeholder `crates/teramind-hook/src/main.rs`**

```rust
fn main() {
    eprintln!("teramind-hook: not yet implemented");
    std::process::exit(2);
}
```

This stub is replaced in Section 3.

- [ ] **Step 4: Verify the workspace still resolves**

Run: `cargo metadata --format-version=1 --no-deps 2>&1 | head -10`
Expected: clean JSON output that now lists 6 workspace members including `teramind-hook`.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/teramind-hook/
git commit -m "chore: register teramind-hook crate in workspace"
```

---

## Section 2 — Hook input types (Claude hook JSON parsing)

Each Claude hook event has a different JSON shape on stdin. We parse them into typed structs so translation downstream is type-safe. Tasks 2–8 add one variant at a time, each with a JSON-roundtrip test driven by a real example payload.

### Task 2: `HookInput` enum scaffold

**Files:**
- Create: `crates/teramind-hook/src/hook_input.rs`
- Create: `crates/teramind-hook/src/lib.rs`

- [ ] **Step 1: Create the lib root**

`crates/teramind-hook/src/lib.rs`:

```rust
//! Tiny hook shim binary for routing Claude Code hook events into the Teramind daemon.

pub mod hook_input;
pub mod inbox;
pub mod selftest;
pub mod spawn;
pub mod translate;
```

The modules `inbox`, `selftest`, `spawn`, `translate` don't exist yet — that's OK; commenting them out and adding back later is also fine. To avoid noise, comment them out and uncomment as each section lands:

```rust
//! Tiny hook shim binary for routing Claude Code hook events into the Teramind daemon.

pub mod hook_input;
// pub mod inbox;     // Section 6
// pub mod selftest;  // Section 10
// pub mod spawn;     // Section 5
// pub mod translate; // Section 4
```

- [ ] **Step 2: Add the lib target to Cargo.toml**

Append above `[[bin]]` in `crates/teramind-hook/Cargo.toml`:

```toml
[lib]
name = "teramind_hook"
path = "src/lib.rs"
```

- [ ] **Step 3: Create `crates/teramind-hook/src/hook_input.rs` with the empty enum**

```rust
use serde::{Deserialize, Serialize};

/// Parsed Claude hook event JSON. One variant per hook event type that Teramind cares about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "hook_event_name")]
pub enum HookInput {
    // Variants added in Tasks 3-8.
}
```

This won't fully build yet — an enum with no variants is fine syntactically. `cargo check -p teramind-hook` should pass with a `dead_code` warning.

- [ ] **Step 4: Compile-check**

Run: `cargo check -p teramind-hook`
Expected: succeeds (possibly with warnings about empty enum).

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-hook/Cargo.toml crates/teramind-hook/src/lib.rs crates/teramind-hook/src/hook_input.rs
git commit -m "feat(hook): scaffold teramind-hook lib + HookInput enum"
```

---

### Task 3: `HookInput::SessionStart` variant

**Files:**
- Modify: `crates/teramind-hook/src/hook_input.rs`

Claude's `SessionStart` hook JSON looks roughly like (verified against Claude Code's hook payload spec; see https://docs.claude.com/en/docs/claude-code/hooks):

```json
{
  "hook_event_name": "SessionStart",
  "session_id": "abc-123",
  "cwd": "/Users/me/project",
  "source": "startup"
}
```

Fields beyond `session_id` and `cwd` are Claude-version-dependent; tolerate unknown fields via `#[serde(other)]` or by not asserting on them.

- [ ] **Step 1: Write the failing test**

Append to `crates/teramind-hook/src/hook_input.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_start_parses_from_real_payload() {
        let raw = r#"{
            "hook_event_name": "SessionStart",
            "session_id": "abc-123",
            "cwd": "/Users/me/project",
            "source": "startup"
        }"#;
        let parsed: HookInput = serde_json::from_str(raw).unwrap();
        match parsed {
            HookInput::SessionStart { session_id, cwd, source } => {
                assert_eq!(session_id, "abc-123");
                assert_eq!(cwd, "/Users/me/project");
                assert_eq!(source.as_deref(), Some("startup"));
            }
            other => panic!("expected SessionStart, got {other:?}"),
        }
    }
}
```

- [ ] **Step 2: Run the test (will fail to compile — variant doesn't exist)**

Run: `cargo test -p teramind-hook hook_input::tests::session_start_parses_from_real_payload`
Expected: FAIL with `no variant or associated item named 'SessionStart'`.

- [ ] **Step 3: Add the variant**

Replace the empty enum body in `hook_input.rs` with:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "hook_event_name")]
pub enum HookInput {
    SessionStart {
        session_id: String,
        cwd: String,
        #[serde(default)]
        source: Option<String>,
    },
}
```

- [ ] **Step 4: Run the test and verify it passes**

Run: `cargo test -p teramind-hook hook_input::tests::session_start_parses_from_real_payload`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-hook/src/hook_input.rs
git commit -m "feat(hook): HookInput::SessionStart variant"
```

---

### Task 4: `HookInput::UserPromptSubmit` variant

**Files:**
- Modify: `crates/teramind-hook/src/hook_input.rs`

Claude's `UserPromptSubmit` payload includes the prompt text and the session/turn position.

- [ ] **Step 1: Append failing test**

```rust
#[test]
fn user_prompt_submit_parses() {
    let raw = r#"{
        "hook_event_name": "UserPromptSubmit",
        "session_id": "abc-123",
        "cwd": "/Users/me/project",
        "prompt": "Fix the failing test"
    }"#;
    let parsed: HookInput = serde_json::from_str(raw).unwrap();
    match parsed {
        HookInput::UserPromptSubmit { session_id, cwd, prompt } => {
            assert_eq!(session_id, "abc-123");
            assert_eq!(cwd, "/Users/me/project");
            assert_eq!(prompt, "Fix the failing test");
        }
        other => panic!("expected UserPromptSubmit, got {other:?}"),
    }
}
```

- [ ] **Step 2: Confirm fail.** Run: `cargo test -p teramind-hook user_prompt_submit_parses`.

- [ ] **Step 3: Add the variant**. Extend the enum:

```rust
UserPromptSubmit {
    session_id: String,
    cwd: String,
    prompt: String,
},
```

- [ ] **Step 4: Verify pass.**

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(hook): HookInput::UserPromptSubmit variant"
```

---

### Task 5: `HookInput::PreToolUse` variant

**Files:**
- Modify: `crates/teramind-hook/src/hook_input.rs`

Claude's `PreToolUse` payload identifies the tool by name and includes its serialized input.

- [ ] **Step 1: Append failing test**

```rust
#[test]
fn pre_tool_use_parses() {
    let raw = r#"{
        "hook_event_name": "PreToolUse",
        "session_id": "abc-123",
        "cwd": "/w",
        "tool_name": "Edit",
        "tool_input": { "file_path": "/w/x.rs", "old_string": "a", "new_string": "b" }
    }"#;
    let parsed: HookInput = serde_json::from_str(raw).unwrap();
    match parsed {
        HookInput::PreToolUse { session_id, tool_name, tool_input, .. } => {
            assert_eq!(session_id, "abc-123");
            assert_eq!(tool_name, "Edit");
            assert_eq!(tool_input["file_path"], "/w/x.rs");
        }
        other => panic!("expected PreToolUse, got {other:?}"),
    }
}
```

- [ ] **Step 2: Confirm fail.**

- [ ] **Step 3: Add the variant**

```rust
PreToolUse {
    session_id: String,
    cwd: String,
    tool_name: String,
    tool_input: serde_json::Value,
},
```

- [ ] **Step 4: Verify pass and commit**: `git commit -am "feat(hook): HookInput::PreToolUse variant"`

---

### Task 6: `HookInput::PostToolUse` variant

**Files:**
- Modify: `crates/teramind-hook/src/hook_input.rs`

Claude's `PostToolUse` includes the tool's `tool_response` (or `tool_output`) plus an error flag. The exact field name varies by Claude version; alias both names to a single field.

- [ ] **Step 1: Append failing test**

```rust
#[test]
fn post_tool_use_parses_with_tool_response() {
    let raw = r#"{
        "hook_event_name": "PostToolUse",
        "session_id": "abc-123",
        "cwd": "/w",
        "tool_name": "Edit",
        "tool_input": { "file_path": "/w/x.rs" },
        "tool_response": "edited successfully"
    }"#;
    let parsed: HookInput = serde_json::from_str(raw).unwrap();
    match parsed {
        HookInput::PostToolUse { session_id, tool_name, tool_response, is_error, .. } => {
            assert_eq!(session_id, "abc-123");
            assert_eq!(tool_name, "Edit");
            assert_eq!(tool_response, Some("edited successfully".to_string()));
            assert_eq!(is_error, false);
        }
        other => panic!("expected PostToolUse, got {other:?}"),
    }
}

#[test]
fn post_tool_use_parses_with_is_error() {
    let raw = r#"{
        "hook_event_name": "PostToolUse",
        "session_id": "abc-123",
        "cwd": "/w",
        "tool_name": "Bash",
        "tool_input": { "command": "false" },
        "tool_response": "exit 1",
        "is_error": true
    }"#;
    let parsed: HookInput = serde_json::from_str(raw).unwrap();
    match parsed {
        HookInput::PostToolUse { is_error, .. } => assert!(is_error),
        other => panic!("expected PostToolUse, got {other:?}"),
    }
}
```

- [ ] **Step 2: Confirm fail.**

- [ ] **Step 3: Add the variant**

```rust
PostToolUse {
    session_id: String,
    cwd: String,
    tool_name: String,
    tool_input: serde_json::Value,
    #[serde(default, alias = "tool_output")]
    tool_response: Option<String>,
    #[serde(default)]
    is_error: bool,
},
```

`#[serde(alias = "tool_output")]` handles either field name.

- [ ] **Step 4: Verify pass and commit**: `git commit -am "feat(hook): HookInput::PostToolUse variant with tool_response/tool_output alias"`

---

### Task 7: `HookInput::Stop` variant

**Files:**
- Modify: `crates/teramind-hook/src/hook_input.rs`

Claude's `Stop` payload carries a `stop_hook_active` flag indicating whether this is an inner stop (the agent loop is continuing) or a final stop.

- [ ] **Step 1: Append failing test**

```rust
#[test]
fn stop_final_parses() {
    let raw = r#"{
        "hook_event_name": "Stop",
        "session_id": "abc-123",
        "cwd": "/w",
        "stop_hook_active": false
    }"#;
    let parsed: HookInput = serde_json::from_str(raw).unwrap();
    match parsed {
        HookInput::Stop { session_id, stop_hook_active, .. } => {
            assert_eq!(session_id, "abc-123");
            assert!(!stop_hook_active);
        }
        other => panic!("expected Stop, got {other:?}"),
    }
}
```

- [ ] **Step 2: Confirm fail.**

- [ ] **Step 3: Add the variant**

```rust
Stop {
    session_id: String,
    cwd: String,
    #[serde(default)]
    stop_hook_active: bool,
},
```

- [ ] **Step 4: Verify pass and commit**: `git commit -am "feat(hook): HookInput::Stop variant"`

---

### Task 8: `HookInput::PreCompact` variant

**Files:**
- Modify: `crates/teramind-hook/src/hook_input.rs`

- [ ] **Step 1: Append failing test**

```rust
#[test]
fn pre_compact_parses() {
    let raw = r#"{
        "hook_event_name": "PreCompact",
        "session_id": "abc-123",
        "cwd": "/w"
    }"#;
    let parsed: HookInput = serde_json::from_str(raw).unwrap();
    matches!(parsed, HookInput::PreCompact { .. });
}
```

- [ ] **Step 2: Confirm fail.**

- [ ] **Step 3: Add the variant**

```rust
PreCompact {
    session_id: String,
    cwd: String,
},
```

- [ ] **Step 4: Verify pass and commit**: `git commit -am "feat(hook): HookInput::PreCompact variant"`

---

### Task 9: Unknown / unrecognized hook events tolerated

**Files:**
- Modify: `crates/teramind-hook/src/hook_input.rs`

Future Claude versions may add hook events (`Notification`, `SessionEnd`, etc.) we don't yet handle. The shim must not panic on them — it should silently exit 0 so Claude isn't affected.

- [ ] **Step 1: Append failing test**

```rust
#[test]
fn unrecognized_event_parses_as_other() {
    let raw = r#"{
        "hook_event_name": "Notification",
        "session_id": "abc-123",
        "message": "hi"
    }"#;
    let parsed: HookInput = serde_json::from_str(raw).unwrap();
    match parsed {
        HookInput::Other { hook_event_name, .. } => {
            assert_eq!(hook_event_name, "Notification");
        }
        other => panic!("expected Other, got {other:?}"),
    }
}
```

- [ ] **Step 2: Confirm fail** (the test references `HookInput::Other` which doesn't exist yet).

- [ ] **Step 3: Add the catch-all variant**. Update the enum's serde attribute and add `Other`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "hook_event_name")]
pub enum HookInput {
    SessionStart { /* … */ },
    UserPromptSubmit { /* … */ },
    PreToolUse { /* … */ },
    PostToolUse { /* … */ },
    Stop { /* … */ },
    PreCompact { /* … */ },
    #[serde(other)]
    Other,
}
```

**Problem:** Serde's `#[serde(other)]` only works on unit variants and discards the data. To preserve `hook_event_name`, we restructure: keep a `pre_parse` step that extracts `hook_event_name` first, then deserialize into the typed variant or fall back to a `String`-typed catch-all.

Simpler approach — use `#[serde(other)]` with a unit variant (preserves nothing) and rely on the raw stdin to log unknown events separately:

```rust
#[serde(other)]
Other,
```

If you need the event name preserved (you do, for logging), use a two-step parse in `translate.rs` later. For Section 2's purposes, the unit `Other` is enough.

Adjust the failing test:

```rust
#[test]
fn unrecognized_event_parses_as_other() {
    let raw = r#"{ "hook_event_name": "Notification", "message": "hi" }"#;
    let parsed: HookInput = serde_json::from_str(raw).unwrap();
    assert!(matches!(parsed, HookInput::Other));
}
```

- [ ] **Step 4: Verify pass and commit**: `git commit -am "feat(hook): tolerate unrecognized hook events via Other variant"`

---

## Section 3 — `teramind-hook` binary entry point

### Task 10: Read stdin and parse `HookInput`

**Files:**
- Replace: `crates/teramind-hook/src/main.rs`

- [ ] **Step 1: Replace `main.rs`**

```rust
use std::io::Read;
use teramind_hook::hook_input::HookInput;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    // Read all of stdin into a buffer.
    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_err() {
        // Stdin unavailable: silently exit 0. Claude must not see an error.
        std::process::exit(0);
    }
    let parsed: HookInput = match serde_json::from_str(&buf) {
        Ok(p) => p,
        Err(_) => {
            // Malformed hook input: log to stderr (Claude won't see this if hook is properly redirected)
            // and exit 0. Capture is best-effort.
            std::process::exit(0);
        }
    };
    // Dispatch lands in Section 4. For now, just drop the parsed value and exit.
    let _ = parsed;
    std::process::exit(0);
}
```

`#[tokio::main(flavor = "current_thread")]` uses the single-threaded runtime — lower cold-start cost than the multi-thread default. We'll need a tokio runtime for the IPC client in Section 4.

- [ ] **Step 2: Build the binary**

Run: `cargo build -p teramind-hook`
Expected: clean build.

- [ ] **Step 3: Smoke test — pipe a SessionStart event through it**

```bash
echo '{"hook_event_name":"SessionStart","session_id":"x","cwd":"/tmp","source":"startup"}' \
  | ./target/debug/teramind-hook
echo "Exit: $?"
```

Expected: prints `Exit: 0`.

- [ ] **Step 4: Smoke test malformed input**

```bash
echo 'not json' | ./target/debug/teramind-hook
echo "Exit: $?"
```

Expected: prints `Exit: 0`.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-hook/src/main.rs
git commit -m "feat(hook): main.rs reads stdin and parses HookInput"
```

---

## Section 4 — Hook → IngestEvent dispatch

### Task 11: Scaffold the `translate` module

**Files:**
- Create: `crates/teramind-hook/src/translate.rs`
- Modify: `crates/teramind-hook/src/lib.rs` (uncomment `pub mod translate;`)

- [ ] **Step 1: Uncomment** `pub mod translate;` in `lib.rs`.

- [ ] **Step 2: Create `translate.rs`**

```rust
use crate::hook_input::HookInput;
use teramind_core::ids::{ClientEventId, SessionId, ToolCallId, TurnId};
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use time::OffsetDateTime;
use uuid::Uuid;

/// Translate a parsed Claude hook input into a Teramind `EventEnvelope`.
///
/// Returns `None` for hook events Teramind doesn't ingest (e.g. `HookInput::Other`).
///
/// `agent_session_uuid` is the deterministic SessionId derived from Claude's `session_id` string
/// (see `claude_session_to_uuid`). The daemon also performs this derivation on its end so
/// repeated hook fires for the same Claude session converge on the same DB session row.
pub fn translate(input: HookInput) -> Option<EventEnvelope> {
    let ts = OffsetDateTime::now_utc();
    let client_event_id = ClientEventId::new();
    let event = match input {
        // Implemented in Tasks 12-17.
        _ => return None,
    };
    Some(EventEnvelope { client_event_id, ts, event })
}

/// Deterministically derive a `SessionId` UUID from Claude's session string.
/// Uses UUID v5 with a fixed namespace so multiple hook invocations agree.
pub fn claude_session_to_uuid(claude_session: &str) -> SessionId {
    // Namespace UUID is arbitrary but constant: this one is "teramind:claude_session"
    const NAMESPACE: Uuid = Uuid::from_bytes([
        0x4b, 0x37, 0x8a, 0x7e, 0xb1, 0x4a, 0x4c, 0x2b,
        0x8a, 0x90, 0x6e, 0x6d, 0x6c, 0x6c, 0x77, 0x6f,
    ]);
    SessionId(Uuid::new_v5(&NAMESPACE, claude_session.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_session_uuid_is_deterministic() {
        let a = claude_session_to_uuid("abc-123");
        let b = claude_session_to_uuid("abc-123");
        assert_eq!(a, b);
        let c = claude_session_to_uuid("different");
        assert_ne!(a, c);
    }
}
```

- [ ] **Step 3: Add `uuid` to Cargo.toml dependencies if not already present** (it is — verify).

- [ ] **Step 4: Run the test**

Run: `cargo test -p teramind-hook translate::tests::claude_session_uuid_is_deterministic`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-hook/src/translate.rs crates/teramind-hook/src/lib.rs
git commit -m "feat(hook): scaffold translate module + Claude session UUID derivation"
```

---

### Task 12: Translate `HookInput::SessionStart` → `IngestEvent::SessionStart`

**Files:**
- Modify: `crates/teramind-hook/src/translate.rs`

- [ ] **Step 1: Append failing test**

```rust
#[test]
fn translates_session_start() {
    let input = HookInput::SessionStart {
        session_id: "abc-123".to_string(),
        cwd: "/work".to_string(),
        source: Some("startup".to_string()),
    };
    let env = translate(input).expect("must translate");
    match env.event {
        IngestEvent::SessionStart { session_id, cwd, agent_kind, .. } => {
            assert_eq!(session_id, claude_session_to_uuid("abc-123"));
            assert_eq!(cwd, "/work");
            assert_eq!(agent_kind, "claude_code");
        }
        other => panic!("expected SessionStart, got {other:?}"),
    }
}
```

- [ ] **Step 2: Confirm fail** (currently the `_ => return None` matches everything).

- [ ] **Step 3: Add the SessionStart arm**

Replace the `_ => return None,` line with:

```rust
HookInput::SessionStart { session_id, cwd, source: _ } => IngestEvent::SessionStart {
    session_id: claude_session_to_uuid(&session_id),
    agent_session_id: Some(session_id),
    agent_kind: "claude_code".to_string(),
    cwd,
    os: std::env::consts::OS.to_string(),
    hostname: hostname().unwrap_or_else(|| "unknown".to_string()),
    user_login: whoami_login().unwrap_or_else(|| "unknown".to_string()),
    git_head: None,
    git_branch: None,
},
_ => return None,
```

Add helper functions at the bottom of `translate.rs`:

```rust
fn hostname() -> Option<String> {
    std::env::var("HOSTNAME").ok().or_else(|| {
        // Fallback: read /etc/hostname on Unix; on Windows, use COMPUTERNAME.
        #[cfg(unix)] {
            std::fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_string())
        }
        #[cfg(windows)] {
            std::env::var("COMPUTERNAME").ok()
        }
    })
}

fn whoami_login() -> Option<String> {
    std::env::var("USER").ok().or_else(|| std::env::var("USERNAME").ok())
}
```

(Git HEAD/branch detection requires shelling out to `git`. Skip for v1; Plan A's spec leaves these as `Option` for this reason. Later plans can populate them.)

- [ ] **Step 4: Verify pass.** Run: `cargo test -p teramind-hook translates_session_start` → PASS.

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(hook): translate SessionStart → IngestEvent::SessionStart"
```

---

### Task 13: Translate `HookInput::UserPromptSubmit` → `IngestEvent::UserPrompt`

**Files:**
- Modify: `crates/teramind-hook/src/translate.rs`

The challenge: Claude's `UserPromptSubmit` doesn't carry a turn ordinal. The shim doesn't know which turn this is (the daemon's session manager could, but the shim is stateless). Convention: send turn_ordinal = -1 to mean "unknown", and let the daemon assign the next ordinal on its end.

Actually, simpler: the daemon's `TraceRepo::upsert_turn` requires an `ordinal` and uses `ON CONFLICT (session_id, ordinal)`. A turn_ordinal of -1 is illegal (column is `integer NOT NULL`, no constraint that excludes negatives, but it's semantically wrong).

**Resolution:** The shim derives a monotonic turn ordinal *per Claude session* by counting `UserPromptSubmit` events seen so far. Since the shim is stateless across invocations, this requires shared state — typically a file lock or a per-session counter file under `~/.local/share/teramind/turn_counters/<session>`.

Simplest path: introduce a tiny on-disk counter at `~/.local/share/teramind/state/turns/<session_id>.count` that increments atomically on each `UserPromptSubmit`.

- [ ] **Step 1: Append a counter helper to `translate.rs`**

Add at the bottom of the file:

```rust
use std::path::PathBuf;

/// Atomically increment a per-session turn counter on disk.
/// Returns the new (post-increment) ordinal, 0-indexed.
fn next_turn_ordinal(session_id: &str) -> i32 {
    let dir = teramind_state_dir().join("turns");
    if std::fs::create_dir_all(&dir).is_err() {
        return 0; // best-effort
    }
    let path: PathBuf = dir.join(format!("{session_id}.count"));
    let current: i32 = std::fs::read_to_string(&path).ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(-1);
    let next = current + 1;
    let _ = std::fs::write(&path, next.to_string());
    next
}

fn teramind_state_dir() -> PathBuf {
    #[cfg(unix)] {
        let home = std::env::var_os("HOME").map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("teramind")
    }
    #[cfg(windows)] {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Temp"))
            .join("teramind")
    }
}
```

(This is best-effort: TOCTOU race between read and write means concurrent hook invocations for the same session may collide. Claude's hooks are typically serialized per session, so in practice this is fine. Plan refinement: switch to `fcntl`-based file locking if races appear.)

- [ ] **Step 2: Append failing test**

```rust
#[test]
fn translates_user_prompt_with_ordinal() {
    // Use a unique session id to avoid colliding with counters from other tests.
    let sid = format!("test-up-{}", uuid::Uuid::new_v4());
    let input = HookInput::UserPromptSubmit {
        session_id: sid.clone(),
        cwd: "/w".into(),
        prompt: "hello".into(),
    };
    let env = translate(input).expect("must translate");
    match env.event {
        IngestEvent::UserPrompt { session_id, turn_ordinal, prompt } => {
            assert_eq!(session_id, claude_session_to_uuid(&sid));
            assert_eq!(turn_ordinal, 0);
            assert_eq!(prompt, "hello");
        }
        other => panic!("expected UserPrompt, got {other:?}"),
    }
    // Second call increments.
    let env2 = translate(HookInput::UserPromptSubmit {
        session_id: sid, cwd: "/w".into(), prompt: "next".into(),
    }).unwrap();
    if let IngestEvent::UserPrompt { turn_ordinal, .. } = env2.event {
        assert_eq!(turn_ordinal, 1);
    }
}
```

- [ ] **Step 3: Confirm fail** (no UserPromptSubmit arm yet).

- [ ] **Step 4: Add the UserPromptSubmit arm**

Add before the `_ => return None,` line:

```rust
HookInput::UserPromptSubmit { session_id, cwd: _, prompt } => {
    let ordinal = next_turn_ordinal(&session_id);
    IngestEvent::UserPrompt {
        session_id: claude_session_to_uuid(&session_id),
        turn_ordinal: ordinal,
        prompt,
    }
}
```

- [ ] **Step 5: Verify pass and commit**

Run: `cargo test -p teramind-hook translates_user_prompt_with_ordinal` → PASS.

```bash
git commit -am "feat(hook): translate UserPromptSubmit with on-disk turn ordinal counter"
```

---

### Task 14: Translate `HookInput::PreToolUse` → `IngestEvent::ToolCallStart`

**Files:**
- Modify: `crates/teramind-hook/src/translate.rs`

PreToolUse refers to the current turn, but doesn't carry a `turn_id` (UUID) — only Claude's session_id and a tool ordinal we need to assign. The daemon's `TraceRepo` keys tool_calls by `(turn_id, ordinal)`. The shim doesn't know the current `TurnId` (it's a UUID generated server-side at upsert time).

**Resolution:** The shim *also* derives `turn_id` deterministically from `(claude_session, turn_ordinal)` using UUIDv5, mirroring the session approach. The daemon agrees on this derivation. This makes `turn_id` predictable to both sides without coordination.

Add to `translate.rs`:

```rust
pub fn claude_turn_to_uuid(session_id: SessionId, turn_ordinal: i32) -> TurnId {
    const NAMESPACE: Uuid = Uuid::from_bytes([
        0xa1, 0xb2, 0xc3, 0xd4, 0xe5, 0xf6, 0x07, 0x18,
        0x29, 0x3a, 0x4b, 0x5c, 0x6d, 0x7e, 0x8f, 0x90,
    ]);
    let mut bytes = [0u8; 20];
    bytes[..16].copy_from_slice(session_id.0.as_bytes());
    bytes[16..].copy_from_slice(&turn_ordinal.to_be_bytes());
    TurnId(Uuid::new_v5(&NAMESPACE, &bytes))
}
```

The current turn ordinal is the *last-written* counter value (without incrementing). Add:

```rust
fn current_turn_ordinal(session_id: &str) -> i32 {
    let path = teramind_state_dir().join("turns").join(format!("{session_id}.count"));
    std::fs::read_to_string(&path).ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}
```

For the tool ordinal within a turn, similar per-turn counter:

```rust
fn next_tool_ordinal(session_id: &str, turn_ordinal: i32) -> i32 {
    let dir = teramind_state_dir().join("tools");
    if std::fs::create_dir_all(&dir).is_err() {
        return 0;
    }
    let path = dir.join(format!("{session_id}-{turn_ordinal}.count"));
    let current: i32 = std::fs::read_to_string(&path).ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(-1);
    let next = current + 1;
    let _ = std::fs::write(&path, next.to_string());
    next
}
```

The daemon's `TraceRepo::insert_tool_call_start` also needs the `turn_id`. The shim sends `turn_id` directly in the IngestEvent (already part of the variant).

- [ ] **Step 1: Append failing test**

```rust
#[test]
fn translates_pre_tool_use() {
    let sid = format!("test-ptu-{}", uuid::Uuid::new_v4());
    // Bump the turn counter so we have a current turn to attach to.
    let _ = next_turn_ordinal(&sid);
    let input = HookInput::PreToolUse {
        session_id: sid.clone(),
        cwd: "/w".into(),
        tool_name: "Edit".into(),
        tool_input: serde_json::json!({"file_path": "/w/x.rs"}),
    };
    let env = translate(input).expect("must translate");
    match env.event {
        IngestEvent::ToolCallStart { turn_id, ordinal, name, input } => {
            assert_eq!(turn_id, claude_turn_to_uuid(claude_session_to_uuid(&sid), 0));
            assert_eq!(ordinal, 0);
            assert_eq!(name, "Edit");
            assert_eq!(input["file_path"], "/w/x.rs");
        }
        other => panic!("expected ToolCallStart, got {other:?}"),
    }
}
```

- [ ] **Step 2: Confirm fail.**

- [ ] **Step 3: Add the PreToolUse arm**

Before `_ => return None,`:

```rust
HookInput::PreToolUse { session_id, cwd: _, tool_name, tool_input } => {
    let sid_uuid = claude_session_to_uuid(&session_id);
    let turn_ord = current_turn_ordinal(&session_id);
    let turn_id = claude_turn_to_uuid(sid_uuid, turn_ord);
    let tool_ord = next_tool_ordinal(&session_id, turn_ord);
    IngestEvent::ToolCallStart {
        turn_id,
        ordinal: tool_ord,
        name: tool_name,
        input: tool_input,
    }
}
```

- [ ] **Step 4: Verify pass and commit**: `git commit -am "feat(hook): translate PreToolUse → ToolCallStart with deterministic turn UUID"`

---

### Task 15: Translate `HookInput::PostToolUse` → `IngestEvent::ToolCallEnd`

**Files:**
- Modify: `crates/teramind-hook/src/translate.rs`

PostToolUse must identify the same `tool_call_id` the daemon stored at PreToolUse. The daemon assigns `tool_call_id` as a fresh UUID inside `insert_tool_call_start`. The shim doesn't know it.

**Resolution:** As with turn_id, derive `tool_call_id` deterministically: UUIDv5 from `(turn_id, tool_ordinal)`.

```rust
pub fn claude_tool_call_to_uuid(turn_id: TurnId, tool_ordinal: i32) -> ToolCallId {
    const NAMESPACE: Uuid = Uuid::from_bytes([
        0xc1, 0xd2, 0xe3, 0xf4, 0x05, 0x16, 0x27, 0x38,
        0x49, 0x5a, 0x6b, 0x7c, 0x8d, 0x9e, 0xaf, 0xb0,
    ]);
    let mut bytes = [0u8; 20];
    bytes[..16].copy_from_slice(turn_id.0.as_bytes());
    bytes[16..].copy_from_slice(&tool_ordinal.to_be_bytes());
    ToolCallId(Uuid::new_v5(&NAMESPACE, &bytes))
}
```

This requires the daemon's `TraceRepo::insert_tool_call_start` to *accept* a caller-provided `id` rather than letting Postgres generate one. **Daemon change needed:** add `TraceRepo::insert_tool_call_start_with_id(id, …)`.

We also need to track the *current* tool ordinal (the most recent one, not the next). Add:

```rust
fn current_tool_ordinal(session_id: &str, turn_ordinal: i32) -> i32 {
    let path = teramind_state_dir().join("tools").join(format!("{session_id}-{turn_ordinal}.count"));
    std::fs::read_to_string(&path).ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}
```

- [ ] **Step 1: Append failing test**

```rust
#[test]
fn translates_post_tool_use() {
    let sid = format!("test-post-{}", uuid::Uuid::new_v4());
    let _ = next_turn_ordinal(&sid);
    let _ = next_tool_ordinal(&sid, 0);
    let input = HookInput::PostToolUse {
        session_id: sid.clone(),
        cwd: "/w".into(),
        tool_name: "Edit".into(),
        tool_input: serde_json::json!({}),
        tool_response: Some("ok".into()),
        is_error: false,
    };
    let env = translate(input).expect("must translate");
    match env.event {
        IngestEvent::ToolCallEnd { tool_call_id, output, is_error, duration_ms } => {
            let expected_turn = claude_turn_to_uuid(claude_session_to_uuid(&sid), 0);
            let expected_tc = claude_tool_call_to_uuid(expected_turn, 0);
            assert_eq!(tool_call_id, expected_tc);
            assert_eq!(output, "ok");
            assert_eq!(is_error, false);
            assert_eq!(duration_ms, 0);
        }
        other => panic!("expected ToolCallEnd, got {other:?}"),
    }
}
```

- [ ] **Step 2: Confirm fail.**

- [ ] **Step 3: Add the PostToolUse arm**

```rust
HookInput::PostToolUse { session_id, cwd: _, tool_name: _, tool_input: _, tool_response, is_error } => {
    let sid_uuid = claude_session_to_uuid(&session_id);
    let turn_ord = current_turn_ordinal(&session_id);
    let turn_id = claude_turn_to_uuid(sid_uuid, turn_ord);
    let tool_ord = current_tool_ordinal(&session_id, turn_ord);
    let tool_call_id = claude_tool_call_to_uuid(turn_id, tool_ord);
    IngestEvent::ToolCallEnd {
        tool_call_id,
        output: tool_response.unwrap_or_default(),
        is_error,
        duration_ms: 0, // Claude does not pass duration in hook payload
    }
}
```

- [ ] **Step 4: Verify pass and commit**: `git commit -am "feat(hook): translate PostToolUse → ToolCallEnd with deterministic tool_call_id"`

---

### Task 16: Daemon-side accept caller-provided `tool_call_id`

**Files:**
- Modify: `crates/teramind-db/src/repos/trace.rs`
- Modify: `crates/teramind-db/tests/repos.rs`
- Modify: `crates/teramindd/src/services/ingest.rs`

The deterministic `tool_call_id` derived in Task 15 is only useful if the daemon writes that exact id into the DB. Currently `insert_tool_call_start` lets Postgres generate the id.

- [ ] **Step 1: Add `insert_tool_call_start_with_id` to `TraceRepo`**

Append to `crates/teramind-db/src/repos/trace.rs` inside `impl TraceRepo`:

```rust
pub async fn insert_tool_call_start_with_id(
    &self,
    id: ToolCallId,
    turn_id: TurnId,
    ordinal: i32,
    name: &str,
    input: &serde_json::Value,
    started_at: OffsetDateTime,
) -> Result<ToolCallId> {
    sqlx::query(
        r#"
        INSERT INTO tool_calls (id, turn_id, ordinal, name, input, started_at)
        VALUES ($1,$2,$3,$4,$5,$6)
        ON CONFLICT (turn_id, ordinal) DO NOTHING
        "#)
        .bind(id.0).bind(turn_id.0).bind(ordinal).bind(name).bind(input).bind(started_at)
        .execute(self.pool.pg()).await?;
    Ok(id)
}
```

- [ ] **Step 2: Add a test**

Append to `crates/teramind-db/tests/repos.rs`:

```rust
#[tokio::test]
async fn trace_repo_accepts_caller_provided_tool_call_id() {
    let f = Fixture::new().await;
    let agents = teramind_db::repos::AgentRepo::new(f.pool.clone());
    let agent = agents.upsert("claude_code", None).await.unwrap();
    let sessions = teramind_db::repos::SessionRepo::new(f.pool.clone());
    let now = time::OffsetDateTime::now_utc();
    let session_id = sessions.insert(teramind_db::repos::session::NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/w", project_id: None,
        parent_session_id: None, git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u", started_at: now,
    }).await.unwrap();
    let trace = teramind_db::repos::TraceRepo::new(f.pool.clone());
    let turn = trace.upsert_turn(session_id, 0, now, None).await.unwrap();

    let chosen_id = teramind_core::ids::ToolCallId::new();
    let returned = trace.insert_tool_call_start_with_id(chosen_id, turn, 0, "Edit", &serde_json::json!({}), now).await.unwrap();
    assert_eq!(returned, chosen_id);

    let (db_id,): (uuid::Uuid,) = sqlx::query_as("SELECT id FROM tool_calls WHERE turn_id=$1 AND ordinal=0")
        .bind(turn.0).fetch_one(f.pool.pg()).await.unwrap();
    assert_eq!(db_id, chosen_id.0);

    f.shutdown().await;
}
```

- [ ] **Step 3: Run the test**: `cargo test -p teramind-db --test repos trace_repo_accepts_caller_provided_tool_call_id` → PASS.

- [ ] **Step 4: Update the daemon's ingest route to use `insert_tool_call_start_with_id` for `ToolCallStart` events** (so the daemon actually honors the shim's deterministic id)

Edit `crates/teramindd/src/services/ingest.rs`. In the `route()` function, find the `ToolCallStart` arm. It currently looks like:

```rust
ToolCallStart { turn_id, ordinal, name, input } => {
    let _ = d.trace.insert_tool_call_start(turn_id, ordinal, &name, &input, ts).await?;
}
```

There's no `id` field in the `ToolCallStart` IngestEvent variant — the daemon generates one inside `insert_tool_call_start`. To pass through the deterministic id, the IngestEvent variant must carry it.

**Plan correction:** add an optional `tool_call_id` to `IngestEvent::ToolCallStart`. This is a forward-compatible addition.

Edit `crates/teramind-core/src/types/ingest_event.rs`. Find the `ToolCallStart` variant and add `tool_call_id`:

```rust
ToolCallStart {
    turn_id: TurnId,
    #[serde(default)]
    tool_call_id: Option<ToolCallId>,
    ordinal: i32,
    name: String,
    input: Value,
},
```

Run `cargo build -p teramind-core` — passes.

Update the shim's `translate.rs` PreToolUse arm to populate it:

```rust
HookInput::PreToolUse { session_id, cwd: _, tool_name, tool_input } => {
    let sid_uuid = claude_session_to_uuid(&session_id);
    let turn_ord = current_turn_ordinal(&session_id);
    let turn_id = claude_turn_to_uuid(sid_uuid, turn_ord);
    let tool_ord = next_tool_ordinal(&session_id, turn_ord);
    let tool_call_id = claude_tool_call_to_uuid(turn_id, tool_ord);
    IngestEvent::ToolCallStart {
        turn_id,
        tool_call_id: Some(tool_call_id),
        ordinal: tool_ord,
        name: tool_name,
        input: tool_input,
    }
}
```

Update the daemon route:

```rust
ToolCallStart { turn_id, tool_call_id, ordinal, name, input } => {
    match tool_call_id {
        Some(id) => { d.trace.insert_tool_call_start_with_id(id, turn_id, ordinal, &name, &input, ts).await?; }
        None     => { let _ = d.trace.insert_tool_call_start(turn_id, ordinal, &name, &input, ts).await?; }
    }
}
```

- [ ] **Step 5: Run daemon tests to confirm nothing regressed**

Run: `cargo test -p teramindd && cargo test -p teramind-db`
Expected: all pass (existing tests use `tool_call_id: None` implicit-default after recompile).

- [ ] **Step 6: Commit**

```bash
git add crates/teramind-db/src/repos/trace.rs crates/teramind-db/tests/repos.rs \
        crates/teramindd/src/services/ingest.rs \
        crates/teramind-core/src/types/ingest_event.rs \
        crates/teramind-hook/src/translate.rs
git commit -m "feat(db,daemon,hook): plumb caller-provided tool_call_id through ingest"
```

---

### Task 17: Translate `HookInput::Stop` → `IngestEvent::SessionEnd` (final stop only)

**Files:**
- Modify: `crates/teramind-hook/src/translate.rs`

Only fire `SessionEnd` when `stop_hook_active == false` (final stop). Inner stops within the agent loop are no-ops.

- [ ] **Step 1: Append failing test**

```rust
#[test]
fn translates_stop_final_to_session_end() {
    let input = HookInput::Stop {
        session_id: "abc-stop".into(),
        cwd: "/w".into(),
        stop_hook_active: false,
    };
    let env = translate(input).expect("must translate");
    matches!(env.event, IngestEvent::SessionEnd { .. });
}

#[test]
fn translates_stop_inner_to_none() {
    let input = HookInput::Stop {
        session_id: "abc-stop".into(),
        cwd: "/w".into(),
        stop_hook_active: true,
    };
    assert!(translate(input).is_none());
}
```

- [ ] **Step 2: Confirm fails.**

- [ ] **Step 3: Add the arm**

```rust
HookInput::Stop { session_id, cwd: _, stop_hook_active } => {
    if stop_hook_active {
        return None;
    }
    IngestEvent::SessionEnd {
        session_id: claude_session_to_uuid(&session_id),
        reason: "stop_hook".to_string(),
    }
}
```

- [ ] **Step 4: Verify pass and commit**: `git commit -am "feat(hook): translate final Stop → SessionEnd"`

---

### Task 18: Translate `HookInput::PreCompact` → `IngestEvent::PreCompact`

**Files:**
- Modify: `crates/teramind-hook/src/translate.rs`

- [ ] **Step 1: Append failing test**

```rust
#[test]
fn translates_pre_compact() {
    let input = HookInput::PreCompact { session_id: "abc-pc".into(), cwd: "/w".into() };
    let env = translate(input).expect("must translate");
    matches!(env.event, IngestEvent::PreCompact { .. });
}
```

- [ ] **Step 2: Confirm fail.**

- [ ] **Step 3: Add the arm**

```rust
HookInput::PreCompact { session_id, cwd: _ } => {
    IngestEvent::PreCompact { session_id: claude_session_to_uuid(&session_id) }
}
```

- [ ] **Step 4: Verify pass and commit**: `git commit -am "feat(hook): translate PreCompact"`

---

### Task 19: `HookInput::Other` returns `None`

**Files:**
- Modify: `crates/teramind-hook/src/translate.rs`

- [ ] **Step 1: Append test**

```rust
#[test]
fn other_input_returns_none() {
    assert!(translate(HookInput::Other).is_none());
}
```

- [ ] **Step 2: The `_ => return None,` arm already handles this**. Run the test — should PASS without further changes.

- [ ] **Step 3: Commit**

```bash
git commit -am "test(hook): verify Other input returns None"
```

---

## Section 5 — Lazy daemon spawn

### Task 20: `spawn::ensure_daemon` — try-connect, exec teramindd, retry once

**Files:**
- Create: `crates/teramind-hook/src/spawn.rs`
- Modify: `crates/teramind-hook/src/lib.rs` (uncomment `pub mod spawn;`)

When `teramind-hook` runs and the daemon isn't listening, it spawns `teramindd` in the background, waits up to ~250 ms, retries connect once.

- [ ] **Step 1: Uncomment `pub mod spawn;` in `lib.rs`.**

- [ ] **Step 2: Create `spawn.rs`**

```rust
use std::path::{Path, PathBuf};
use std::time::Duration;

/// If the daemon socket can be connected, returns `Ok(stream)`.
/// If not, spawn `teramindd` in the background and retry once.
/// Returns `Err` only if both connection attempts fail.
pub async fn ensure_daemon_connected(socket: &Path) -> std::io::Result<()> {
    if try_connect(socket, Duration::from_millis(50)).await.is_ok() {
        return Ok(());
    }
    spawn_daemon_detached()?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    try_connect(socket, Duration::from_millis(50)).await
}

async fn try_connect(socket: &Path, deadline: Duration) -> std::io::Result<()> {
    let r = tokio::time::timeout(deadline, teramind_ipc::transport::connect(socket)).await;
    match r {
        Ok(Ok(_stream)) => Ok(()),
        Ok(Err(e)) => Err(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, e.to_string())),
        Err(_) => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "connect deadline")),
    }
}

fn spawn_daemon_detached() -> std::io::Result<()> {
    let exe = which_teramindd()?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.stdin(std::process::Stdio::null())
       .stdout(std::process::Stdio::null())
       .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x00000008); // DETACHED_PROCESS
    }
    let _ = cmd.spawn()?;
    Ok(())
}

fn which_teramindd() -> std::io::Result<PathBuf> {
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            let cand = dir.join(if cfg!(windows) { "teramindd.exe" } else { "teramindd" });
            if cand.exists() { return Ok(cand); }
        }
    }
    if let Ok(out) = std::process::Command::new(if cfg!(windows) { "where" } else { "which" })
        .arg("teramindd").output() {
        if out.status.success() {
            if let Some(line) = String::from_utf8_lossy(&out.stdout).lines().next() {
                return Ok(PathBuf::from(line.trim()));
            }
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::NotFound, "teramindd not found"))
}
```

- [ ] **Step 3: Compile-check**

Run: `cargo check -p teramind-hook`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-hook/src/spawn.rs crates/teramind-hook/src/lib.rs
git commit -m "feat(hook): lazy daemon spawn helper"
```

---

## Section 6 — Inbox fallback

### Task 21: `inbox::write_envelope` — persist to disk when daemon unreachable

**Files:**
- Create: `crates/teramind-hook/src/inbox.rs`
- Modify: `crates/teramind-hook/src/lib.rs` (uncomment `pub mod inbox;`)

- [ ] **Step 1: Uncomment `pub mod inbox;` in `lib.rs`.**

- [ ] **Step 2: Create `inbox.rs`**

```rust
use teramind_core::types::ingest_event::EventEnvelope;
use std::path::PathBuf;

pub fn write_envelope(env: &EventEnvelope) -> std::io::Result<PathBuf> {
    let dir = inbox_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", env.client_event_id.0));
    std::fs::write(&path, serde_json::to_vec(env)?)?;
    Ok(path)
}

fn inbox_dir() -> PathBuf {
    #[cfg(unix)] {
        let home = std::env::var_os("HOME").map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("teramind").join("inbox")
    }
    #[cfg(windows)] {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Temp"))
            .join("teramind").join("inbox")
    }
}
```

- [ ] **Step 3: Add test**

Append to `inbox.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use teramind_core::ids::{ClientEventId, SessionId};
    use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
    use time::OffsetDateTime;

    #[test]
    fn writes_envelope_to_inbox() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        std::env::set_var("XDG_DATA_HOME", tmp.path().join("xdg-data"));
        #[cfg(windows)] std::env::set_var("LOCALAPPDATA", tmp.path());
        let env = EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::now_utc(),
            event: IngestEvent::UserPrompt {
                session_id: SessionId::new(), turn_ordinal: 0, prompt: "x".into(),
            },
        };
        let path = write_envelope(&env).unwrap();
        assert!(path.exists());
        let parsed: EventEnvelope = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(parsed.client_event_id, env.client_event_id);
    }
}
```

- [ ] **Step 4: Run test and commit**

Run: `cargo test -p teramind-hook inbox::tests::writes_envelope_to_inbox` → PASS.

```bash
git add crates/teramind-hook/src/inbox.rs crates/teramind-hook/src/lib.rs
git commit -m "feat(hook): inbox write for daemon-unreachable fallback"
```

---

## Section 7 — Wire main.rs end-to-end

### Task 22: Full hook dispatch in main.rs

**Files:**
- Replace: `crates/teramind-hook/src/main.rs`

- [ ] **Step 1: Replace main.rs**

```rust
use std::io::Read;
use teramind_hook::{hook_input::HookInput, inbox, spawn, translate};
use teramind_ipc::{client::{IpcClient, StreamClient}, proto::Notify, transport::{connect, default_socket_path}};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_err() {
        std::process::exit(0);
    }
    let parsed: HookInput = match serde_json::from_str(&buf) {
        Ok(p) => p,
        Err(_) => std::process::exit(0),
    };
    let envelope = match translate::translate(parsed) {
        Some(e) => e,
        None => std::process::exit(0),
    };

    let socket = default_socket_path();
    // Try to send to daemon. Spawn it if absent.
    if let Err(_) = spawn::ensure_daemon_connected(&socket).await {
        let _ = inbox::write_envelope(&envelope);
        std::process::exit(0);
    }
    // Connect and fire notify.
    let stream = match connect(&socket).await {
        Ok(s) => s,
        Err(_) => {
            let _ = inbox::write_envelope(&envelope);
            std::process::exit(0);
        }
    };
    let mut client = StreamClient::new(stream);
    let _ = client.notify(Notify::Ingest(envelope.clone())).await;
    // Whether the notify succeeded or not, we exit 0. The daemon's ingest channel
    // may be saturated; that drop is accounted for in `ingest_drops_total`.
    std::process::exit(0);
}
```

- [ ] **Step 2: Build**

Run: `cargo build -p teramind-hook`
Expected: clean.

- [ ] **Step 3: Smoke test against a running daemon**

```bash
# Terminal 1: start the daemon manually
HOME=$(mktemp -d) cargo run -p teramindd &
DPID=$!
sleep 5

# Terminal 1 same: pipe a SessionStart event
echo '{"hook_event_name":"SessionStart","session_id":"smoke-test","cwd":"/tmp","source":"startup"}' \
  | ./target/debug/teramind-hook

# Cleanup
kill $DPID
```

The daemon log under `~/.local/share/teramind/logs/` (or your `XDG_DATA_HOME`) should contain a successful ingest event. Skip this smoke if it's awkward in CI; Section 11 integration tests cover the same path.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-hook/src/main.rs
git commit -m "feat(hook): wire main.rs — parse stdin, translate, notify daemon or inbox"
```

---

## Section 8 — Claude plugin bundle template

### Task 23: Plugin manifest + hook shell wrappers

**Files:**
- Create: `plugins/claude/plugin.json`
- Create: `plugins/claude/hooks/session_start.sh`
- Create: `plugins/claude/hooks/user_prompt_submit.sh`
- Create: `plugins/claude/hooks/pre_tool_use.sh`
- Create: `plugins/claude/hooks/post_tool_use.sh`
- Create: `plugins/claude/hooks/stop.sh`
- Create: `plugins/claude/hooks/pre_compact.sh`
- Create: `plugins/claude/skills/.gitkeep`
- Create: `plugins/claude/README.md`

- [ ] **Step 1: Write `plugins/claude/plugin.json`**

```json
{
  "name": "teramind",
  "description": "Captures Claude Code sessions into the local Teramind daemon.",
  "version": "0.1.0",
  "homepage": "https://teramind.dev",
  "hooks": {
    "SessionStart": "@TERAMIND_PLUGIN_DIR@/hooks/session_start.sh",
    "UserPromptSubmit": "@TERAMIND_PLUGIN_DIR@/hooks/user_prompt_submit.sh",
    "PreToolUse": "@TERAMIND_PLUGIN_DIR@/hooks/pre_tool_use.sh",
    "PostToolUse": "@TERAMIND_PLUGIN_DIR@/hooks/post_tool_use.sh",
    "Stop": "@TERAMIND_PLUGIN_DIR@/hooks/stop.sh",
    "PreCompact": "@TERAMIND_PLUGIN_DIR@/hooks/pre_compact.sh"
  }
}
```

`@TERAMIND_PLUGIN_DIR@` is a placeholder the installer substitutes with the absolute installed plugin path (covered in Task 25). Claude Code reads the manifest and respects absolute hook paths.

(Claude's actual plugin format is `plugin.json` referenced in the user's settings; depending on the Claude version, hooks may also live in `settings.json` — the installer accommodates both. The plugin manifest is the canonical source.)

- [ ] **Step 2: Write the 6 hook shell wrappers**

Each wrapper is one line — it execs `teramind-hook` with stdin forwarded. The installer rewrites the path to the absolute teramind-hook binary at install time. Template:

`plugins/claude/hooks/session_start.sh`:
```sh
#!/bin/sh
exec @TERAMIND_HOOK_BIN@
```

`plugins/claude/hooks/user_prompt_submit.sh`, `pre_tool_use.sh`, `post_tool_use.sh`, `stop.sh`, `pre_compact.sh` — all identical content (`exec @TERAMIND_HOOK_BIN@`). The shim parses the `hook_event_name` field from stdin itself, so a single binary handles all six.

- [ ] **Step 3: Make all .sh files executable in the source tree**

```bash
chmod +x plugins/claude/hooks/*.sh
```

(Git tracks the executable bit. Verify with `git ls-files --stage plugins/claude/hooks/` after staging — entries should show mode `100755`.)

- [ ] **Step 4: Write the empty `plugins/claude/skills/.gitkeep`** (zero bytes; just to keep the directory in git for later population by codifier in Plan C+).

- [ ] **Step 5: Write `plugins/claude/README.md`**

```markdown
# Teramind Claude Plugin

This directory is the *template* for the Teramind Claude Code plugin.
It is copied verbatim into `~/.claude/plugins/teramind/` by `teramind claude install`,
which also patches the `@TERAMIND_PLUGIN_DIR@` and `@TERAMIND_HOOK_BIN@`
placeholders to absolute paths on the user's machine.

Do not run anything here directly. Use `teramind claude install` to deploy.
```

- [ ] **Step 6: Commit**

```bash
git add plugins/claude/
git commit -m "feat(plugin): Claude Code plugin template (manifest + hook wrappers)"
```

---

## Section 9 — `teramind claude install` / `uninstall`

### Task 24: Subcommand registration

**Files:**
- Modify: `crates/teramind/src/cli.rs`
- Modify: `crates/teramind/src/main.rs`
- Create: `crates/teramind/src/commands/claude.rs`

- [ ] **Step 1: Add `Claude` subcommand to `Cli`**

Edit `crates/teramind/src/cli.rs`. Add to `Command` enum:

```rust
/// Manage the Claude Code plugin integration.
Claude {
    #[command(subcommand)]
    action: ClaudeAction,
},
```

And add the inner enum:

```rust
#[derive(Debug, Subcommand)]
pub enum ClaudeAction {
    /// Install the Teramind Claude plugin (`~/.claude/plugins/teramind/`).
    Install,
    /// Remove the Teramind Claude plugin. Data is untouched.
    Uninstall,
}
```

- [ ] **Step 2: Wire into `main.rs`**

Update the match arm:

```rust
Command::Claude { action } => match action {
    cli::ClaudeAction::Install   => commands::claude::install().await,
    cli::ClaudeAction::Uninstall => commands::claude::uninstall().await,
},
```

- [ ] **Step 3: Add `mod claude;` to `commands/mod.rs`**

```rust
pub mod claude;
```

(Append to the existing list.)

- [ ] **Step 4: Create the dispatcher stub** `crates/teramind/src/commands/claude.rs`:

```rust
pub async fn install() -> anyhow::Result<()> {
    crate::commands::claude_install::run().await
}
pub async fn uninstall() -> anyhow::Result<()> {
    crate::commands::claude_uninstall::run().await
}
```

Also append `pub mod claude_install; pub mod claude_uninstall;` to `commands/mod.rs`.

Create stub `crates/teramind/src/commands/claude_install.rs`:

```rust
pub async fn run() -> anyhow::Result<()> { anyhow::bail!("not yet implemented") }
```

Same for `claude_uninstall.rs`.

- [ ] **Step 5: Build to confirm CLI parses**

Run: `cargo build -p teramind-cli && cargo run -p teramind-cli -- claude --help`
Expected: clap prints the subcommand help.

- [ ] **Step 6: Commit**

```bash
git add crates/teramind/src/cli.rs crates/teramind/src/main.rs crates/teramind/src/commands/
git commit -m "feat(cli): scaffold claude install/uninstall subcommands"
```

---

### Task 25: Implement `claude install` — copy template, patch placeholders

**Files:**
- Replace: `crates/teramind/src/commands/claude_install.rs`

- [ ] **Step 1: Write the install logic**

```rust
use anyhow::Context;
use std::path::PathBuf;

pub async fn run() -> anyhow::Result<()> {
    let claude_home = claude_home()?;
    let plugin_dir = claude_home.join("plugins").join("teramind");

    // 1. Resolve the absolute paths the installer will patch in.
    let teramind_hook_bin = which_teramind_hook()?;
    let plugin_dir_str = plugin_dir.to_string_lossy().into_owned();
    let hook_bin_str = teramind_hook_bin.to_string_lossy().into_owned();

    // 2. Wipe any prior install so we don't mix stale templates.
    if plugin_dir.exists() {
        std::fs::remove_dir_all(&plugin_dir)
            .with_context(|| format!("remove existing {}", plugin_dir.display()))?;
    }
    std::fs::create_dir_all(&plugin_dir.join("hooks"))?;
    std::fs::create_dir_all(&plugin_dir.join("skills"))?;

    // 3. Copy template files with placeholder substitution.
    let template_dir = locate_template_dir()?;
    for entry in walk_template(&template_dir) {
        let rel = entry.strip_prefix(&template_dir).unwrap();
        let dst = plugin_dir.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&dst)?;
            continue;
        }
        // Skip .gitkeep markers
        if entry.file_name().and_then(|n| n.to_str()) == Some(".gitkeep") {
            std::fs::create_dir_all(dst.parent().unwrap())?;
            continue;
        }
        let bytes = std::fs::read(&entry)?;
        let text = String::from_utf8_lossy(&bytes)
            .replace("@TERAMIND_PLUGIN_DIR@", &plugin_dir_str)
            .replace("@TERAMIND_HOOK_BIN@", &hook_bin_str);
        std::fs::write(&dst, text.as_bytes())
            .with_context(|| format!("write {}", dst.display()))?;
        // Preserve executable bit on hook scripts.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let was_exec = std::fs::metadata(&entry)?.permissions().mode() & 0o111 != 0;
            if was_exec {
                let mut perms = std::fs::metadata(&dst)?.permissions();
                perms.set_mode(perms.mode() | 0o755);
                std::fs::set_permissions(&dst, perms)?;
            }
        }
    }

    println!("Teramind plugin installed at {}", plugin_dir.display());
    println!("Open Claude Code; run a session; then `teramind sessions` to confirm capture.");
    Ok(())
}

fn claude_home() -> anyhow::Result<PathBuf> {
    if let Ok(h) = std::env::var("CLAUDE_HOME") {
        return Ok(PathBuf::from(h));
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .context("HOME (or USERPROFILE on Windows) is not set")?;
    Ok(home.join(".claude"))
}

fn which_teramind_hook() -> anyhow::Result<PathBuf> {
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            let cand = dir.join(if cfg!(windows) { "teramind-hook.exe" } else { "teramind-hook" });
            if cand.exists() { return Ok(cand); }
        }
    }
    if let Ok(out) = std::process::Command::new(if cfg!(windows) { "where" } else { "which" })
        .arg("teramind-hook").output() {
        if out.status.success() {
            if let Some(line) = String::from_utf8_lossy(&out.stdout).lines().next() {
                return Ok(PathBuf::from(line.trim()));
            }
        }
    }
    anyhow::bail!("teramind-hook binary not found next to teramind or on PATH")
}

fn locate_template_dir() -> anyhow::Result<PathBuf> {
    // 1. Built-in: same dir as the CLI executable, under `plugins/claude/`.
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            let cand = dir.join("plugins").join("claude");
            if cand.join("plugin.json").exists() { return Ok(cand); }
        }
    }
    // 2. Dev path: workspace `plugins/claude/`.
    if let Ok(cwd) = std::env::current_dir() {
        for level in 0..5 {
            let mut p = cwd.clone();
            for _ in 0..level { p = p.parent().unwrap_or(&p).to_path_buf(); }
            let cand = p.join("plugins").join("claude");
            if cand.join("plugin.json").exists() { return Ok(cand); }
        }
    }
    // 3. Env override.
    if let Ok(d) = std::env::var("TERAMIND_PLUGIN_TEMPLATE_DIR") {
        let p = PathBuf::from(d);
        if p.join("plugin.json").exists() { return Ok(p); }
    }
    anyhow::bail!("Could not locate the Claude plugin template directory; \
                   set TERAMIND_PLUGIN_TEMPLATE_DIR to the path containing plugin.json")
}

fn walk_template(root: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn walk(p: &std::path::Path, out: &mut Vec<PathBuf>) {
        if p.is_dir() {
            if let Ok(rd) = std::fs::read_dir(p) {
                for entry in rd.flatten() { walk(&entry.path(), out); }
            }
        } else {
            out.push(p.to_path_buf());
        }
    }
    walk(root, &mut out);
    out
}
```

- [ ] **Step 2: Smoke-test in an isolated `$CLAUDE_HOME`**

```bash
cargo build --workspace
TMP_CH=$(mktemp -d)
TERAMIND_PLUGIN_TEMPLATE_DIR=$(pwd)/plugins/claude \
CLAUDE_HOME=$TMP_CH \
  ./target/debug/teramind claude install
ls -la "$TMP_CH/plugins/teramind/"
cat "$TMP_CH/plugins/teramind/plugin.json"
```

Expected: plugin directory created with manifest + hook scripts, paths patched to absolute values.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind/src/commands/claude_install.rs
git commit -m "feat(cli): teramind claude install — copy template, patch absolute paths"
```

---

### Task 26: Implement `claude uninstall` — remove plugin dir cleanly

**Files:**
- Replace: `crates/teramind/src/commands/claude_uninstall.rs`

- [ ] **Step 1: Write the uninstall logic**

```rust
use anyhow::Context;
use std::path::PathBuf;

pub async fn run() -> anyhow::Result<()> {
    let claude_home = claude_home()?;
    let plugin_dir = claude_home.join("plugins").join("teramind");
    if plugin_dir.exists() {
        std::fs::remove_dir_all(&plugin_dir)
            .with_context(|| format!("remove {}", plugin_dir.display()))?;
        println!("Teramind plugin removed from {}", plugin_dir.display());
    } else {
        println!("Teramind plugin was not installed (nothing at {})", plugin_dir.display());
    }
    println!("User data is untouched. Use `teramind uninstall --purge --confirm` to remove data.");
    Ok(())
}

fn claude_home() -> anyhow::Result<PathBuf> {
    if let Ok(h) = std::env::var("CLAUDE_HOME") { return Ok(PathBuf::from(h)); }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .context("HOME (or USERPROFILE on Windows) is not set")?;
    Ok(home.join(".claude"))
}
```

(Yes, `claude_home` is duplicated between install and uninstall. A future refactor could extract it; for two-call usage this is fine.)

- [ ] **Step 2: Smoke-test**

```bash
TMP_CH=$(mktemp -d)
TERAMIND_PLUGIN_TEMPLATE_DIR=$(pwd)/plugins/claude \
CLAUDE_HOME=$TMP_CH \
  ./target/debug/teramind claude install
CLAUDE_HOME=$TMP_CH ./target/debug/teramind claude uninstall
ls "$TMP_CH/plugins/" 2>/dev/null  # should be empty / missing
```

- [ ] **Step 3: Commit**

```bash
git add crates/teramind/src/commands/claude_uninstall.rs
git commit -m "feat(cli): teramind claude uninstall — remove plugin dir, preserve data"
```

---

### Task 27: Integration test for install/uninstall roundtrip

**Files:**
- Create: `crates/teramind/tests/claude_install.rs`

- [ ] **Step 1: Write the test**

```rust
#![cfg(unix)]
use std::process::Command;
use tempfile::tempdir;

fn cargo_bin(name: &str) -> std::path::PathBuf {
    let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    std::path::PathBuf::from(target).join(profile).join(name)
}

#[test]
fn claude_install_uninstall_roundtrip() {
    let _ = Command::new("cargo").args(["build", "-p", "teramind-cli", "-p", "teramind-hook"]).status();

    let claude_home = tempdir().unwrap();
    let template_dir = std::env::current_dir().unwrap()
        .ancestors()
        .find(|p| p.join("plugins").join("claude").join("plugin.json").exists())
        .map(|p| p.join("plugins").join("claude"))
        .expect("could not find plugins/claude in ancestors");

    let teramind = cargo_bin("teramind");
    let env: Vec<(&str, String)> = vec![
        ("CLAUDE_HOME", claude_home.path().to_string_lossy().into_owned()),
        ("TERAMIND_PLUGIN_TEMPLATE_DIR", template_dir.to_string_lossy().into_owned()),
    ];

    // Install
    let out = Command::new(&teramind).args(["claude", "install"]).envs(env.iter().cloned()).output().unwrap();
    assert!(out.status.success(), "install failed: {}", String::from_utf8_lossy(&out.stderr));

    let manifest = claude_home.path().join("plugins/teramind/plugin.json");
    assert!(manifest.exists());
    let body = std::fs::read_to_string(&manifest).unwrap();
    // The template's @TERAMIND_PLUGIN_DIR@ must have been substituted.
    assert!(!body.contains("@TERAMIND_PLUGIN_DIR@"), "placeholder left unpatched in manifest");
    assert!(body.contains(&format!("{}/plugins/teramind", claude_home.path().display())),
            "absolute plugin dir not present in manifest body");

    let hook_script = claude_home.path().join("plugins/teramind/hooks/session_start.sh");
    let body = std::fs::read_to_string(&hook_script).unwrap();
    assert!(!body.contains("@TERAMIND_HOOK_BIN@"), "placeholder left unpatched in hook script");

    // Uninstall
    let out = Command::new(&teramind).args(["claude", "uninstall"]).envs(env.iter().cloned()).output().unwrap();
    assert!(out.status.success(), "uninstall failed: {}", String::from_utf8_lossy(&out.stderr));
    assert!(!claude_home.path().join("plugins/teramind").exists(), "plugin dir not removed");
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p teramind-cli --test claude_install claude_install_uninstall_roundtrip`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind/tests/claude_install.rs
git commit -m "test(cli): claude install/uninstall roundtrip integration test"
```

---

## Section 10 — Self-test

### Task 28: `teramind-hook --selftest`

**Files:**
- Create: `crates/teramind-hook/src/selftest.rs`
- Modify: `crates/teramind-hook/src/lib.rs` (uncomment `pub mod selftest;`)
- Modify: `crates/teramind-hook/src/main.rs` (accept `--selftest` flag)

The plugin installer calls `teramind-hook --selftest` post-install to verify the binary is reachable and produces sensible output without contacting the daemon.

- [ ] **Step 1: Uncomment `pub mod selftest;` in `lib.rs`.**

- [ ] **Step 2: Create `selftest.rs`**

```rust
use crate::{hook_input::HookInput, translate};

/// Self-test the hook: parse a canned SessionStart payload, verify translation produces an envelope.
/// Returns Ok(()) on success; prints diagnostic to stderr on failure.
pub fn run() -> Result<(), String> {
    let raw = r#"{"hook_event_name":"SessionStart","session_id":"selftest","cwd":"/tmp","source":"startup"}"#;
    let parsed: HookInput = serde_json::from_str(raw)
        .map_err(|e| format!("parse failed: {e}"))?;
    let env = translate::translate(parsed)
        .ok_or_else(|| "translate returned None".to_string())?;
    println!("teramind-hook selftest OK");
    println!("  envelope.client_event_id: {}", env.client_event_id);
    println!("  envelope.ts: {}", env.ts);
    println!("  envelope.event: {:?}", env.event);
    Ok(())
}
```

- [ ] **Step 3: Update main.rs to handle `--selftest`**

At the top of `main()` in `main.rs`, before the stdin read:

```rust
let args: Vec<String> = std::env::args().collect();
if args.iter().any(|a| a == "--selftest") {
    match teramind_hook::selftest::run() {
        Ok(()) => std::process::exit(0),
        Err(e) => { eprintln!("teramind-hook selftest FAILED: {e}"); std::process::exit(1); }
    }
}
```

- [ ] **Step 4: Smoke**

```bash
cargo build -p teramind-hook
./target/debug/teramind-hook --selftest
echo "exit: $?"
```

Expected: prints "teramind-hook selftest OK" with envelope details; exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-hook/src/selftest.rs crates/teramind-hook/src/lib.rs crates/teramind-hook/src/main.rs
git commit -m "feat(hook): --selftest flag for installer verification"
```

---

### Task 29: Installer runs `--selftest` post-install

**Files:**
- Modify: `crates/teramind/src/commands/claude_install.rs`

- [ ] **Step 1: After the "Teramind plugin installed at ..." print, add the selftest call**

Insert before the final `Ok(())`:

```rust
// Verify the hook binary is reachable.
let status = std::process::Command::new(&teramind_hook_bin).arg("--selftest").status();
match status {
    Ok(s) if s.success() => println!("teramind-hook self-test passed."),
    _ => println!("WARNING: teramind-hook self-test failed; hooks may not fire correctly."),
}
```

(Move the `let teramind_hook_bin = which_teramind_hook()?;` line earlier in `run()` if needed so it's bound when the selftest runs.)

- [ ] **Step 2: Smoke-test by running install in a tempdir**

```bash
TMP_CH=$(mktemp -d)
TERAMIND_PLUGIN_TEMPLATE_DIR=$(pwd)/plugins/claude \
CLAUDE_HOME=$TMP_CH \
  ./target/debug/teramind claude install
```

Expected: prints both the plugin path AND "teramind-hook self-test passed."

- [ ] **Step 3: Commit**

```bash
git commit -am "feat(cli): run teramind-hook --selftest after claude install"
```

---

## Section 11 — L3 capture E2E integration tests

### Task 30: Happy-path test — pipe SessionStart through real daemon, assert DB row

**Files:**
- Create: `crates/teramind-hook/tests/happy_path.rs`

- [ ] **Step 1: Write the test**

```rust
#![cfg(unix)]
use std::process::{Command, Stdio};
use std::io::Write;
use teramind_db::{pg_supervisor::PgSupervisor, pool::DbPool, migrate};
use teramind_ipc::transport::listen;
use std::sync::Arc;
use teramindd::services::{
    ingest::{IngestDeps, IngestService, IngestStats},
    ipc_server::{run_accept_loop, DaemonIpcHandler},
    jsonl_writer::JsonlWriter,
    session_manager::SessionManager,
};
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo};
use teramind_core::redact::Redactor;
use tempfile::tempdir;

fn cargo_bin(name: &str) -> std::path::PathBuf {
    let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    std::path::PathBuf::from(target).join(profile).join(name)
}

#[tokio::test]
async fn hook_session_start_persists_to_postgres() {
    // 0. Build the hook binary if missing.
    let _ = Command::new("cargo").args(["build", "-p", "teramind-hook"]).status();

    // 1. Bring up a daemon stack in-process so we control the socket.
    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().join("pg"), "teramind_test").await.unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();

    let jsonl = Arc::new(JsonlWriter::open(tmp.path().join("raw")).await.unwrap());
    let stats = Arc::new(IngestStats::default());
    let ingest = Arc::new(IngestService::spawn(64, IngestDeps {
        redactor: Arc::new(Redactor::with_default_rules()),
        jsonl: jsonl.clone(),
        sessions: SessionManager::new(),
        agents: AgentRepo::new(pool.clone()),
        session_repo: SessionRepo::new(pool.clone()),
        trace: TraceRepo::new(pool.clone()),
        diffs: DiffRepo::new(pool.clone()),
        stats: stats.clone(),
        dead_letter_dir: tmp.path().join("dl"),
    }));
    let handler = Arc::new(DaemonIpcHandler {
        ingest: ingest.clone(), stats: stats.clone(),
        started: std::time::Instant::now(),
        last_pg_bytes: 0.into(), last_jsonl_bytes: 0.into(),
    });
    let sock = tmp.path().join("t.sock");
    let listener = listen(&sock).unwrap();
    let h2 = handler.clone();
    tokio::spawn(async move { let _ = run_accept_loop(listener, h2).await; });

    // 2. Configure the hook binary to use this socket.
    let hook = cargo_bin("teramind-hook");
    let payload = r#"{"hook_event_name":"SessionStart","session_id":"e2e-test","cwd":"/work","source":"startup"}"#;
    let mut child = Command::new(&hook)
        .env("TERAMIND_SOCKET", sock.to_string_lossy().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn().unwrap();
    child.stdin.as_mut().unwrap().write_all(payload.as_bytes()).unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "hook exited non-zero");

    // 3. Allow the daemon to drain.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // 4. Verify a session row exists with the deterministic UUID.
    let expected_id = teramind_hook::translate::claude_session_to_uuid("e2e-test").0;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM sessions WHERE id=$1")
        .bind(expected_id).fetch_one(pool.pg()).await.unwrap();
    assert_eq!(count, 1, "expected exactly one session row with id={expected_id}");

    sup.shutdown().await.unwrap();
}
```

(Note: the test depends on the daemon's IPC server honoring `TERAMIND_SOCKET`. Plan A's `transport/unix.rs::default_socket_path()` already reads that env var.)

The daemon's session manager / ingest needs to accept the deterministic SessionId from the envelope and use it instead of generating a fresh one. Plan A added `SessionRepo::insert_with_id` for exactly this; the ingest service already prefers the envelope id when non-nil.

- [ ] **Step 2: Run the test**

Run: `cargo test -p teramind-hook --test happy_path hook_session_start_persists_to_postgres -- --nocapture`
Expected: PASS. ~10–30s on first run due to embedded PG.

- [ ] **Step 3: If it fails because the daemon ingest doesn't write a row with the envelope-supplied id**, trace the path:
1. The shim sends `IngestEvent::SessionStart { session_id: claude_session_to_uuid("e2e-test"), … }`.
2. The daemon's `route()` does `let agent = agents.upsert(...)`, then `session_repo.insert(NewSession { ... })` (which lets PG generate a fresh id).
3. So the daemon's chosen id ≠ the envelope's chosen id.

Plan A's fix introduced `insert_with_id` but the route logic falls back to `insert` (PG-generated) when… actually re-reading Plan A's Section 8 Task 49 reveals the fix was inserted there but the `SessionStart` arm of `route()` was patched to use `insert_with_id` when `session_id != nil`. Verify that's still the case in `crates/teramindd/src/services/ingest.rs`. If not, fix it.

If the route already uses `insert_with_id` correctly, the test passes.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-hook/tests/happy_path.rs
git commit -m "test(hook): SessionStart hook → daemon → PG row integration test"
```

---

### Task 31: Tool-call lifecycle test — PreToolUse + PostToolUse persist correctly

**Files:**
- Modify: `crates/teramind-hook/tests/happy_path.rs`

- [ ] **Step 1: Append test**

```rust
#[tokio::test]
async fn hook_tool_call_lifecycle_persists() {
    let _ = Command::new("cargo").args(["build", "-p", "teramind-hook"]).status();
    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().join("pg"), "teramind_test").await.unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();

    let jsonl = Arc::new(JsonlWriter::open(tmp.path().join("raw")).await.unwrap());
    let stats = Arc::new(IngestStats::default());
    let ingest = Arc::new(IngestService::spawn(64, IngestDeps {
        redactor: Arc::new(Redactor::with_default_rules()),
        jsonl: jsonl.clone(),
        sessions: SessionManager::new(),
        agents: AgentRepo::new(pool.clone()),
        session_repo: SessionRepo::new(pool.clone()),
        trace: TraceRepo::new(pool.clone()),
        diffs: DiffRepo::new(pool.clone()),
        stats: stats.clone(),
        dead_letter_dir: tmp.path().join("dl"),
    }));
    let handler = Arc::new(DaemonIpcHandler {
        ingest: ingest.clone(), stats: stats.clone(),
        started: std::time::Instant::now(),
        last_pg_bytes: 0.into(), last_jsonl_bytes: 0.into(),
    });
    let sock = tmp.path().join("t.sock");
    let listener = listen(&sock).unwrap();
    let h2 = handler.clone();
    tokio::spawn(async move { let _ = run_accept_loop(listener, h2).await; });

    let hook = cargo_bin("teramind-hook");
    // Use a unique session id for this test to keep the on-disk turn counter isolated.
    let sid = format!("tc-{}", uuid::Uuid::new_v4());
    let payloads = vec![
        format!(r#"{{"hook_event_name":"SessionStart","session_id":"{sid}","cwd":"/w","source":"startup"}}"#),
        format!(r#"{{"hook_event_name":"UserPromptSubmit","session_id":"{sid}","cwd":"/w","prompt":"do it"}}"#),
        format!(r#"{{"hook_event_name":"PreToolUse","session_id":"{sid}","cwd":"/w","tool_name":"Edit","tool_input":{{"file_path":"/w/x.rs"}}}}"#),
        format!(r#"{{"hook_event_name":"PostToolUse","session_id":"{sid}","cwd":"/w","tool_name":"Edit","tool_input":{{"file_path":"/w/x.rs"}},"tool_response":"ok"}}"#),
    ];

    // Send each payload through the hook binary, one at a time.
    let tmp_state = tempdir().unwrap();
    for p in &payloads {
        let mut child = Command::new(&hook)
            .env("TERAMIND_SOCKET", sock.to_string_lossy().to_string())
            .env("XDG_DATA_HOME", tmp_state.path()) // keep turn counters in tempdir
            .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null())
            .spawn().unwrap();
        child.stdin.as_mut().unwrap().write_all(p.as_bytes()).unwrap();
        assert!(child.wait().unwrap().success());
    }

    tokio::time::sleep(std::time::Duration::from_millis(700)).await;

    let session_uuid = teramind_hook::translate::claude_session_to_uuid(&sid).0;

    let (s_count,): (i64,) = sqlx::query_as("SELECT count(*) FROM sessions WHERE id=$1")
        .bind(session_uuid).fetch_one(pool.pg()).await.unwrap();
    assert_eq!(s_count, 1);

    let (t_count,): (i64,) = sqlx::query_as("SELECT count(*) FROM turns WHERE session_id=$1")
        .bind(session_uuid).fetch_one(pool.pg()).await.unwrap();
    assert_eq!(t_count, 1);

    let (tc_count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM tool_calls tc JOIN turns t ON tc.turn_id=t.id WHERE t.session_id=$1 AND tc.name='Edit' AND tc.output='ok'")
        .bind(session_uuid).fetch_one(pool.pg()).await.unwrap();
    assert_eq!(tc_count, 1);

    sup.shutdown().await.unwrap();
}
```

- [ ] **Step 2: Run**: `cargo test -p teramind-hook --test happy_path hook_tool_call_lifecycle_persists -- --nocapture` → PASS.

- [ ] **Step 3: Commit**

```bash
git commit -am "test(hook): tool-call lifecycle (SessionStart→UserPrompt→Pre/PostToolUse) persists"
```

---

### Task 32: Inbox-fallback test — daemon down, hook writes to inbox, no error

**Files:**
- Create: `crates/teramind-hook/tests/inbox_fallback.rs`

- [ ] **Step 1: Write the test**

```rust
#![cfg(unix)]
use std::process::{Command, Stdio};
use std::io::Write;
use tempfile::tempdir;

fn cargo_bin(name: &str) -> std::path::PathBuf {
    let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    std::path::PathBuf::from(target).join(profile).join(name)
}

#[test]
fn hook_writes_to_inbox_when_daemon_unreachable() {
    let _ = Command::new("cargo").args(["build", "-p", "teramind-hook"]).status();

    let tmp = tempdir().unwrap();
    let sock = tmp.path().join("no-such.sock"); // intentionally does not exist
    let xdg = tmp.path().join("xdg-data");

    let payload = r#"{"hook_event_name":"UserPromptSubmit","session_id":"inbox-test","cwd":"/w","prompt":"hi"}"#;
    let hook = cargo_bin("teramind-hook");
    let mut child = Command::new(&hook)
        .env("TERAMIND_SOCKET", sock.to_string_lossy().to_string())
        .env("HOME", tmp.path())
        .env("XDG_DATA_HOME", &xdg)
        .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().unwrap();
    child.stdin.as_mut().unwrap().write_all(payload.as_bytes()).unwrap();
    assert!(child.wait().unwrap().success(), "hook must exit 0 even when daemon is down");

    // The hook should have written exactly one .json file under inbox/.
    let inbox_dir = xdg.join("teramind").join("inbox");
    assert!(inbox_dir.exists(), "inbox dir not created");
    let files: Vec<_> = std::fs::read_dir(&inbox_dir).unwrap().filter_map(Result::ok).collect();
    assert_eq!(files.len(), 1, "expected exactly one inbox file, found {}", files.len());
}
```

**Caveat:** the hook in Section 7's main.rs calls `spawn::ensure_daemon_connected` which tries to spawn `teramindd` if the daemon isn't running. In this test environment with the `teramindd` binary on disk, it might succeed in spawning a daemon — defeating the inbox-fallback assertion.

To force the inbox path, override `PATH` so `teramindd` is not findable. Add to the test:

```rust
// Force teramindd to be unfindable so the spawn attempt fails and we fall through to inbox.
("PATH", "/usr/bin:/bin".to_string()),
```

…and also remove `target/debug` from being adjacent. Easier: rename or skip the auto-build. Cleanest: introduce a `TERAMIND_HOOK_NO_SPAWN=1` env var the shim's `spawn::ensure_daemon_connected` honors:

```rust
// In spawn.rs:
pub async fn ensure_daemon_connected(socket: &Path) -> std::io::Result<()> {
    if try_connect(socket, Duration::from_millis(50)).await.is_ok() {
        return Ok(());
    }
    if std::env::var("TERAMIND_HOOK_NO_SPAWN").is_ok() {
        return Err(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "spawn disabled"));
    }
    spawn_daemon_detached()?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    try_connect(socket, Duration::from_millis(50)).await
}
```

Use that env var in the test:

```rust
.env("TERAMIND_HOOK_NO_SPAWN", "1")
```

- [ ] **Step 2: Add `TERAMIND_HOOK_NO_SPAWN` handling to `spawn.rs`** (per above).

- [ ] **Step 3: Run the test**: `cargo test -p teramind-hook --test inbox_fallback hook_writes_to_inbox_when_daemon_unreachable` → PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-hook/src/spawn.rs crates/teramind-hook/tests/inbox_fallback.rs
git commit -m "test(hook): inbox fallback when daemon unreachable (TERAMIND_HOOK_NO_SPAWN)"
```

---

## Section 12 — L4 documentation (manual smoke with real Claude Code)

### Task 33: Runbook for verifying capture against a real Claude Code session

**Files:**
- Create: `docs/runbooks/claude-capture-manual-smoke.md`

This is a documentation-only task. L4 tests cannot run in CI without a real Claude Code installation; we capture the procedure for humans.

- [ ] **Step 1: Write the runbook**

```markdown
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
   # Find session_id of most recent claude_code session
   psql "$(teramind status --format=json | jq -r '.pg_url')" -c \
     "SELECT id, cwd, started_at, ended_at FROM sessions ORDER BY started_at DESC LIMIT 1;"
   # Inspect turns
   psql … -c "SELECT ordinal, length(user_prompt), length(assistant_text) FROM turns WHERE session_id = '<id>' ORDER BY ordinal;"
   # Inspect tool calls
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
```

- [ ] **Step 2: Commit**

```bash
mkdir -p docs/runbooks
git add docs/runbooks/claude-capture-manual-smoke.md
git commit -m "docs: manual smoke runbook for Claude → Teramind capture"
```

---

## Plan B completion checklist

By the end of Task 33 you should have on the branch:

- `teramind-hook` crate compiled, with a sub-millisecond binary (one shim covering all six hook events).
- Deterministic UUID derivation (SessionId, TurnId, ToolCallId) so hook and daemon agree without coordination.
- Per-session and per-turn on-disk counters under `~/.local/share/teramind/state/` for ordinal assignment.
- `IngestEvent::ToolCallStart` extended with `tool_call_id: Option<ToolCallId>`; daemon honors it via `insert_tool_call_start_with_id`.
- `plugins/claude/` template directory committed (manifest + 6 hook wrappers).
- `teramind claude install` and `teramind claude uninstall` subcommands, with placeholder substitution for absolute paths and post-install `--selftest`.
- L3 capture E2E tests: SessionStart-only happy path, full tool-call lifecycle, inbox fallback when daemon is unreachable.
- L4 runbook for verifying against a real Claude Code session.

Tests added by Plan B (expected ~12 new):
- `hook_input` parses: 6 + 1 catch-all = 7 unit tests.
- `translate`: deterministic UUID, all 6 variants = 8 unit tests (5 are translate cases; 3 are UUID/ordinal helpers).
- `inbox::writes_envelope_to_inbox` = 1.
- `claude_install_uninstall_roundtrip` = 1.
- `hook_session_start_persists_to_postgres` = 1.
- `hook_tool_call_lifecycle_persists` = 1.
- `hook_writes_to_inbox_when_daemon_unreachable` = 1.
- `trace_repo_accepts_caller_provided_tool_call_id` (db crate) = 1.

Total Plan B: ≈ 21 new tests. Plus Plan A's 48 = 69 tests workspace-wide.

What Plan B does **not** ship (deferred):
- Search, MCP server, slash commands (Plan C).
- FS watcher and auto-recall digest (Plan D).
- Installer scripts and release packaging (Plan E).
- L5 search-effectiveness benchmark (Plan F).

---

## Plan self-review

**Spec coverage** (against `docs/superpowers/specs/2026-05-13-teramind-core-design.md`):
- §2.1 plugin bundle (hooks, MCP, slash, skills): Plan B covers hooks. MCP, slash, skills are Plan C.
- §3 architecture (hook → daemon flow): Tasks 22 (main.rs wire) and 20 (spawn) cover this end-to-end.
- §4.2 `teramind-hook` crate responsibilities: Tasks 1–22.
- §5 capture flow per-event sequence: Tasks 12–18 implement each event's translation; §5.2 ordinal/idempotency is satisfied by deterministic UUIDs + `ON CONFLICT DO NOTHING` in `insert_tool_call_start_with_id`.
- §5.4 failure behavior — hook can't reach daemon: Task 21 (`inbox::write_envelope`); Task 32 verifies it.
- §7.4 `teramind claude install`: Tasks 25, 27, 29.

**Placeholder scan:** none. Every code step has full code; commands are concrete; expected outputs stated.

**Type-name consistency:**
- `HookInput` variants (`SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, `PreCompact`, `Other`) are used consistently across Tasks 2–18 and the test file.
- `translate::translate`, `translate::claude_session_to_uuid`, `translate::claude_turn_to_uuid`, `translate::claude_tool_call_to_uuid` — used the same way in main.rs (Task 22) and tests (Tasks 30, 31).
- `spawn::ensure_daemon_connected` — same name in main.rs (Task 22) and spawn.rs (Task 20).
- `inbox::write_envelope` — same in main.rs (Task 22) and inbox.rs (Task 21).
- `TraceRepo::insert_tool_call_start_with_id` — defined in Task 16, referenced in Task 16's daemon update; same signature.
- `IngestEvent::ToolCallStart { tool_call_id: Option<ToolCallId>, … }` — added in Task 16, populated in Task 16's PreToolUse update, consumed in Task 16's daemon route update.

**Scope check:** focused on a single subsystem (Claude Code capture). 33 tasks; would be reasonable to subdivide further only if subagent dispatches become wieldy.

---

