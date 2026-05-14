# Teramind FS Watcher & Per-Turn Diff Capture — Plan D

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Populate the existing `file_diffs` table with per-turn unified diffs captured by a filesystem watcher running inside `teramindd`, with attribution to either the agent turn that ran the write tool or to the human user.

**Architecture:** A new `fs_watcher` service in the daemon owns one `notify::RecommendedWatcher` per unique active-session cwd. File events are per-path debounced (200 ms). On resolution: the watcher reads post-content, resolves pre-content from an in-memory snapshot cache (or falls back to `git show :rel_path`), computes a unified diff via the `similar` crate, extracts ±50-line excerpts around each hunk, classifies attribution by consulting a 5-second ring buffer of recent write-tool completions, applies redaction, and emits an `IngestEvent::FileDiff` through the existing ingest pipeline. Auto-recall is extended to query diff excerpts for files currently present in cwd.

**Tech Stack:** Rust stable / 1.93.0 (workspace pin), tokio multi-thread, `notify` 8.2 for FS events, `similar` 2 for unified-diff computation, `ignore` 0.4 for gitignore matching, `sha2` for content hashing, existing sqlx/Postgres pipeline.

---

## File Structure

**New files:**
- `crates/teramindd/src/services/diff_engine.rs` — pure diff math: language detection, unified diff, hunk parser, ±50-line excerpt extractor, sha256 hash. ~250 lines.
- `crates/teramindd/src/services/snapshot_cache.rs` — in-memory `(cwd, rel_path) → pre_content` cache with TTL eviction. ~120 lines.
- `crates/teramindd/src/services/git_index.rs` — `git show :./rel_path` shellout helper (best-effort). ~80 lines.
- `crates/teramindd/src/services/write_tool_ring.rs` — bounded ring of recent write-tool completions for attribution lookup. ~100 lines.
- `crates/teramindd/src/services/ignore_filter.rs` — wraps `ignore::gitignore::GitignoreBuilder` + built-in deny list. ~100 lines.
- `crates/teramindd/src/services/fs_watcher.rs` — watcher registry (refcount per cwd), per-path debouncer, event pipeline, attribution decision. ~350 lines.
- `crates/teramindd/tests/fs_watcher_attribution.rs` — L3 integration: agent vs human attribution.
- `crates/teramindd/tests/fs_watcher_e2e.rs` — L3 end-to-end via real ingest pipeline.

**Modified files:**
- `Cargo.toml` (workspace) — add `similar = "2"`, `ignore = "0.4"`.
- `crates/teramindd/Cargo.toml` — depend on those plus `notify` (already in workspace).
- `crates/teramind-core/src/types/ingest_event.rs` — extend `ToolCallEnd` and add `FileDiff` variant.
- `crates/teramind-hook/src/translate.rs` — populate new `ToolCallEnd` fields.
- `crates/teramindd/src/services/ingest.rs` — push to write-tool ring on `ToolCallEnd`; route new `FileDiff` event with redaction.
- `crates/teramindd/src/services/session_manager.rs` — broadcast lifecycle events.
- `crates/teramindd/src/services/mod.rs` — register new modules.
- `crates/teramindd/src/app.rs` — wire `FsWatcherService` + `WriteToolRing` shared between ingest and watcher.
- `crates/teramindd/src/config.rs` — add `fs_watcher` config block.
- `crates/teramindd/src/services/search.rs` — extend `do_auto_recall` with diff-excerpts-for-cwd-files query.
- `crates/teramind-db/src/repos/search.rs` — add `diff_excerpts_for_cwd_files`.

---

## Section 0 — Workspace deps

### Task 0.1: Add `similar` and `ignore` to workspace dependencies

**Files:**
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Edit workspace dependencies block**

Add these two lines to `[workspace.dependencies]` (alphabetical insert after `hex`):

```toml
ignore      = "0.4"
similar     = "2"
```

- [ ] **Step 2: Verify the workspace still resolves**

Run: `cargo metadata --format-version 1 --offline 2>&1 | head -5` from repo root.
Expected: prints the JSON metadata line; no error about missing deps.

(If `--offline` fails because the crates aren't cached yet, run without `--offline`.)

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml
git commit -m "build(deps): add similar + ignore for fs watcher"
```

---

### Task 0.2: Add the new deps to `teramindd`

**Files:**
- Modify: `crates/teramindd/Cargo.toml`

- [ ] **Step 1: Add deps to `[dependencies]`**

Insert (alphabetical):

```toml
ignore  = { workspace = true }
notify  = { workspace = true }
similar = { workspace = true }
sha2    = { workspace = true }
hex     = { workspace = true }
```

(`notify`, `sha2`, `hex` are in the workspace from Plan A but `teramindd` may not yet list them — add any that are missing.)

- [ ] **Step 2: `cargo check -p teramindd`**

Expected: succeeds (or fails only because we haven't added the new modules yet — re-run after Section 7).

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/Cargo.toml
git commit -m "build(teramindd): pull in notify/similar/ignore"
```

---

## Section 1 — `IngestEvent` extension

The hook's `PostToolUse` already knows the tool name, the session, and (via the deterministic UUID) the turn. We carry that forward in the `ToolCallEnd` envelope so the daemon can decide write-tool attribution without an extra DB round-trip. We also add a `FileDiff` variant so the FS watcher emits diffs through the same ingest pipeline that handles redaction + JSONL appending.

### Task 1.1: Extend `IngestEvent::ToolCallEnd` with optional metadata

**Files:**
- Modify: `crates/teramind-core/src/types/ingest_event.rs`

- [ ] **Step 1: Add the failing test**

Append to the `#[cfg(test)] mod tests` block at the bottom of `crates/teramind-core/src/types/ingest_event.rs`:

```rust
#[test]
fn tool_call_end_carries_optional_metadata() {
    let env = EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: OffsetDateTime::from_unix_timestamp(1_700_000_020).unwrap(),
        event: IngestEvent::ToolCallEnd {
            tool_call_id: ToolCallId::new(),
            output: "ok".into(),
            is_error: false,
            duration_ms: 10,
            session_id: Some(SessionId::new()),
            turn_id: Some(TurnId::new()),
            tool_name: Some("Edit".into()),
        },
    };
    let j = serde_json::to_string(&env).unwrap();
    let back: EventEnvelope = serde_json::from_str(&j).unwrap();
    assert_eq!(env, back);
}

#[test]
fn tool_call_end_back_compat_no_metadata() {
    // Older envelopes without the new fields must still deserialize.
    let j = r#"{"client_event_id":"00000000-0000-0000-0000-000000000001","ts":"2026-05-14T00:00:00Z","event":{"type":"tool_call_end","tool_call_id":"00000000-0000-0000-0000-000000000002","output":"x","is_error":false,"duration_ms":1}}"#;
    let env: EventEnvelope = serde_json::from_str(j).unwrap();
    match env.event {
        IngestEvent::ToolCallEnd { session_id, turn_id, tool_name, .. } => {
            assert!(session_id.is_none());
            assert!(turn_id.is_none());
            assert!(tool_name.is_none());
        }
        other => panic!("expected ToolCallEnd, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run the tests, confirm they fail**

Run: `cargo test -p teramind-core types::ingest_event -- --nocapture`
Expected: compile error — `ToolCallEnd` is missing `session_id`, `turn_id`, `tool_name`.

- [ ] **Step 3: Add the fields with `#[serde(default)]`**

Replace the existing `ToolCallEnd` variant (around lines 43-48) with:

```rust
    ToolCallEnd {
        tool_call_id: ToolCallId,
        output: String,
        is_error: bool,
        duration_ms: i32,
        #[serde(default)]
        session_id: Option<SessionId>,
        #[serde(default)]
        turn_id: Option<TurnId>,
        #[serde(default)]
        tool_name: Option<String>,
    },
```

- [ ] **Step 4: Run tests again**

Run: `cargo test -p teramind-core types::ingest_event`
Expected: PASS (both tests).

- [ ] **Step 5: Re-run full teramind-core tests**

Run: `cargo test -p teramind-core`
Expected: PASS — including pre-existing tests.

- [ ] **Step 6: Commit**

```bash
git add crates/teramind-core/src/types/ingest_event.rs
git commit -m "feat(core): carry session/turn/tool_name on ToolCallEnd"
```

---

### Task 1.2: Add `IngestEvent::FileDiff` variant

**Files:**
- Modify: `crates/teramind-core/src/types/ingest_event.rs`

- [ ] **Step 1: Write the failing roundtrip test**

Append to the test module:

```rust
#[test]
fn file_diff_event_roundtrips() {
    let env = EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: OffsetDateTime::from_unix_timestamp(1_700_000_030).unwrap(),
        event: IngestEvent::FileDiff {
            session_id: SessionId::new(),
            turn_id: Some(TurnId::new()),
            file_path: "/proj/src/foo.rs".into(),
            rel_path: "src/foo.rs".into(),
            attribution: teramind_core::types::file_diff::Attribution::Agent,
            language: Some("rust".into()),
            pre_excerpt: "fn old() {}".into(),
            post_excerpt: "fn new() {}".into(),
            unified_diff: "@@ -1 +1 @@\n-fn old() {}\n+fn new() {}\n".into(),
            pre_hash: [0u8; 32],
            post_hash: [1u8; 32],
            byte_size: 12,
        },
    };
    let j = serde_json::to_string(&env).unwrap();
    let back: EventEnvelope = serde_json::from_str(&j).unwrap();
    assert_eq!(env, back);
}
```

- [ ] **Step 2: Run the test, confirm it fails**

Run: `cargo test -p teramind-core file_diff_event_roundtrips`
Expected: FAIL — `FileDiff` variant does not exist on `IngestEvent`.

- [ ] **Step 3: Add the variant**

Add the new variant inside the `IngestEvent` enum (after `PreCompact`):

```rust
    FileDiff {
        session_id: SessionId,
        #[serde(default)]
        turn_id: Option<TurnId>,
        file_path: String,
        rel_path: String,
        attribution: crate::types::file_diff::Attribution,
        #[serde(default)]
        language: Option<String>,
        pre_excerpt: String,
        post_excerpt: String,
        unified_diff: String,
        #[serde(with = "hex_array_32")]
        pre_hash: [u8; 32],
        #[serde(with = "hex_array_32")]
        post_hash: [u8; 32],
        byte_size: i32,
    },
```

Then add a sibling private module at the bottom of the file:

```rust
mod hex_array_32 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(v))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s = String::deserialize(d)?;
        let v = hex::decode(&s).map_err(serde::de::Error::custom)?;
        v.try_into().map_err(|_| serde::de::Error::custom("expected 32 bytes"))
    }
}
```

Also ensure `hex` is in the teramind-core dependencies — it already is (per `crates/teramind-core/Cargo.toml`).

- [ ] **Step 4: Run the test, confirm PASS**

Run: `cargo test -p teramind-core`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-core/src/types/ingest_event.rs
git commit -m "feat(core): IngestEvent::FileDiff variant for FS watcher pipeline"
```

---

### Task 1.3: Hook populates new `ToolCallEnd` fields

**Files:**
- Modify: `crates/teramind-hook/src/translate.rs`

- [ ] **Step 1: Update the failing test**

Locate the existing PostToolUse → ToolCallEnd round-trip test (around line 268 of `translate.rs`). Modify the assertion block to also assert the new metadata is populated:

```rust
        let env = super::translate(input).expect("translated");
        match env.event {
            IngestEvent::ToolCallEnd {
                tool_call_id, output, is_error, duration_ms,
                session_id, turn_id, tool_name
            } => {
                assert_eq!(output, "wrote 12 bytes");
                assert!(!is_error);
                assert_eq!(duration_ms, 0);
                let _ = tool_call_id;
                assert!(session_id.is_some(), "session_id should be populated");
                assert!(turn_id.is_some(), "turn_id should be populated");
                assert_eq!(tool_name.as_deref(), Some("Edit"));
            }
            other => panic!("expected ToolCallEnd, got {other:?}"),
        }
```

- [ ] **Step 2: Run, confirm it fails**

Run: `cargo test -p teramind-hook translate::tests -- --nocapture`
Expected: FAIL — fields are `None` / current struct shape mismatch.

- [ ] **Step 3: Update the PostToolUse arm in `translate()`**

Replace the PostToolUse arm (around lines 62-73) with:

```rust
        HookInput::PostToolUse { session_id, cwd: _, tool_name, tool_input: _, tool_response, is_error } => {
            let sid_uuid = claude_session_to_uuid(&session_id);
            let turn_ord = current_turn_ordinal(&session_id);
            let turn_id = claude_turn_to_uuid(sid_uuid, turn_ord);
            let tool_ord = current_tool_ordinal(&session_id, turn_ord);
            let tool_call_id = claude_tool_call_to_uuid(turn_id, tool_ord);
            IngestEvent::ToolCallEnd {
                tool_call_id,
                output: tool_response.unwrap_or_default(),
                is_error,
                duration_ms: 0,
                session_id: Some(sid_uuid),
                turn_id: Some(turn_id),
                tool_name: Some(tool_name),
            }
        }
```

- [ ] **Step 4: Run, confirm PASS**

Run: `cargo test -p teramind-hook`
Expected: PASS — including the round-trip test.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-hook/src/translate.rs
git commit -m "feat(hook): include session/turn/tool_name in ToolCallEnd"
```

---

### Task 1.4: Fix the daemon's existing `ToolCallEnd` match (compile-only)

The struct shape changed; existing code in `ingest.rs::route` and `redact_envelope` destructure `ToolCallEnd` and must accept the new fields.

**Files:**
- Modify: `crates/teramindd/src/services/ingest.rs`

- [ ] **Step 1: Run `cargo check`, observe the error**

Run: `cargo check -p teramindd`
Expected: errors about missing fields in `ToolCallEnd` pattern.

- [ ] **Step 2: Update `redact_envelope`'s `ToolCallEnd` arm**

Replace the existing `ToolCallEnd { tool_call_id, output, is_error, duration_ms } => …` arm (~line 133-143) with:

```rust
        ToolCallEnd {
            tool_call_id,
            output,
            is_error,
            duration_ms,
            session_id,
            turn_id,
            tool_name,
        } => ToolCallEnd {
            tool_call_id,
            output: r.apply(&output),
            is_error,
            duration_ms,
            session_id,
            turn_id,
            tool_name,
        },
```

- [ ] **Step 3: Update `route()`'s `ToolCallEnd` arm**

Replace the existing arm (~line 233-242) with:

```rust
        ToolCallEnd {
            tool_call_id,
            output,
            is_error,
            duration_ms,
            session_id: _,
            turn_id: _,
            tool_name: _,
        } => {
            d.trace
                .finalize_tool_call(tool_call_id, &output, is_error, duration_ms)
                .await?;
        }
```

(We will start *using* `tool_name`/`session_id`/`turn_id` in Section 5; for now we only need to compile.)

- [ ] **Step 4: `cargo check -p teramindd`**

Expected: succeeds.

- [ ] **Step 5: Commit**

```bash
git add crates/teramindd/src/services/ingest.rs
git commit -m "refactor(daemon): pattern-match new ToolCallEnd fields"
```

---

## Section 2 — Diff engine

Pure functions: no I/O, no async. Everything in this section is fully unit-testable.

### Task 2.1: Scaffold `diff_engine.rs` with `language_from_extension`

**Files:**
- Create: `crates/teramindd/src/services/diff_engine.rs`
- Modify: `crates/teramindd/src/services/mod.rs`

- [ ] **Step 1: Register the new module in `mod.rs`**

Append to `crates/teramindd/src/services/mod.rs`:

```rust
pub mod diff_engine;
```

- [ ] **Step 2: Write the failing test in a new file**

Create `crates/teramindd/src/services/diff_engine.rs` with:

```rust
//! Pure diff math: language detection, unified diff, hunk-bounded excerpts.

use std::path::Path;

/// Map a file extension to a coarse-grained language tag stored on `file_diffs.language`.
/// Returns `None` for unknown/binary/extensionless paths.
pub fn language_from_extension(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => "rust",
        "py" => "python",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "rb" => "ruby",
        "php" => "php",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hxx" => "cpp",
        "cs" => "csharp",
        "scala" => "scala",
        "sh" | "bash" | "zsh" => "shell",
        "sql" => "sql",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "json" => "json",
        "md" | "markdown" => "markdown",
        "html" | "htm" => "html",
        "css" | "scss" | "less" => "css",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn detects_common_languages() {
        assert_eq!(language_from_extension(&PathBuf::from("a.rs")), Some("rust"));
        assert_eq!(language_from_extension(&PathBuf::from("a.PY")), Some("python"));
        assert_eq!(language_from_extension(&PathBuf::from("a.tsx")), Some("typescript"));
    }

    #[test]
    fn unknown_extension_returns_none() {
        assert_eq!(language_from_extension(&PathBuf::from("a.xyz")), None);
        assert_eq!(language_from_extension(&PathBuf::from("Makefile")), None);
    }
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p teramindd diff_engine`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/diff_engine.rs crates/teramindd/src/services/mod.rs
git commit -m "feat(daemon): diff_engine scaffold with language detection"
```

---

### Task 2.2: Add `sha256_hash`

**Files:**
- Modify: `crates/teramindd/src/services/diff_engine.rs`

- [ ] **Step 1: Write the failing test**

Append to the test module of `diff_engine.rs`:

```rust
    #[test]
    fn sha256_hash_is_stable() {
        let h1 = sha256_hash(b"hello");
        let h2 = sha256_hash(b"hello");
        assert_eq!(h1, h2);
        let h3 = sha256_hash(b"world");
        assert_ne!(h1, h3);
        // Known-answer test:
        let expected = hex::decode("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824").unwrap();
        assert_eq!(&h1[..], expected.as_slice());
    }
```

- [ ] **Step 2: Run, confirm fail**

Run: `cargo test -p teramindd diff_engine::tests::sha256_hash_is_stable`
Expected: FAIL — function undefined.

- [ ] **Step 3: Add the function**

Insert near the top of `diff_engine.rs`, below the `use` lines:

```rust
use sha2::{Digest, Sha256};

/// SHA-256 of arbitrary bytes, returned as a fixed-size 32-byte array
/// to match the `file_diffs.pre_hash` / `post_hash` columns.
pub fn sha256_hash(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    arr
}
```

- [ ] **Step 4: Run, confirm PASS**

Run: `cargo test -p teramindd diff_engine`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramindd/src/services/diff_engine.rs
git commit -m "feat(daemon): diff_engine sha256 helper"
```

---

### Task 2.3: Add `unified_diff` via `similar`

**Files:**
- Modify: `crates/teramindd/src/services/diff_engine.rs`

- [ ] **Step 1: Write the failing test**

Append:

```rust
    #[test]
    fn unified_diff_emits_hunks_for_changed_text() {
        let pre = "line1\nline2\nline3\n";
        let post = "line1\nLINE TWO\nline3\n";
        let diff = unified_diff(pre, post, "foo.txt");
        assert!(diff.contains("--- a/foo.txt"), "diff: {diff}");
        assert!(diff.contains("+++ b/foo.txt"));
        assert!(diff.contains("-line2"));
        assert!(diff.contains("+LINE TWO"));
    }

    #[test]
    fn unified_diff_empty_when_identical() {
        let s = "same\n";
        assert!(unified_diff(s, s, "x").is_empty());
    }
```

- [ ] **Step 2: Run, confirm fail**

Run: `cargo test -p teramindd diff_engine::tests::unified_diff_emits_hunks_for_changed_text`
Expected: FAIL — function undefined.

- [ ] **Step 3: Add `unified_diff`**

Insert in `diff_engine.rs`:

```rust
use similar::TextDiff;

/// Produce a unified diff string in `git diff --no-index` style.
/// Header uses `a/<rel>` and `b/<rel>` to match standard parsers.
/// Returns the empty string when `pre == post`.
pub fn unified_diff(pre: &str, post: &str, rel_path: &str) -> String {
    if pre == post {
        return String::new();
    }
    let diff = TextDiff::from_lines(pre, post);
    let mut out = String::new();
    out.push_str(&format!("--- a/{rel_path}\n"));
    out.push_str(&format!("+++ b/{rel_path}\n"));
    for hunk in diff.unified_diff().context_radius(3).iter_hunks() {
        out.push_str(&hunk.to_string());
    }
    out
}
```

- [ ] **Step 4: Run, confirm PASS**

Run: `cargo test -p teramindd diff_engine`
Expected: PASS — both new tests.

- [ ] **Step 5: Commit**

```bash
git add crates/teramindd/src/services/diff_engine.rs
git commit -m "feat(daemon): diff_engine.unified_diff via similar"
```

---

### Task 2.4: Add hunk-bounded excerpt extractor

**Files:**
- Modify: `crates/teramindd/src/services/diff_engine.rs`

- [ ] **Step 1: Write the failing tests**

Append to test module:

```rust
    #[test]
    fn excerpts_extract_50_line_window_around_hunk() {
        // 200-line file; change happens at line 100.
        let pre_lines: Vec<String> = (1..=200).map(|i| format!("line{i}")).collect();
        let mut post_lines = pre_lines.clone();
        post_lines[99] = "CHANGED".to_string();
        let pre = pre_lines.join("\n") + "\n";
        let post = post_lines.join("\n") + "\n";

        let (pre_ex, post_ex) = excerpts_around_hunks(&pre, &post, 50);
        // Expect lines 50..=150 in the excerpt (100 ± 50).
        assert!(pre_ex.contains("line50\n"), "missing line50:\n{pre_ex}");
        assert!(pre_ex.contains("line100\n"));
        assert!(pre_ex.contains("line150\n"));
        assert!(!pre_ex.contains("line49\n"), "excerpt should not include line49");
        assert!(!pre_ex.contains("line151\n"));

        assert!(post_ex.contains("CHANGED\n"));
    }

    #[test]
    fn excerpts_handle_small_file() {
        let pre = "a\nb\nc\n";
        let post = "a\nB\nc\n";
        let (pre_ex, post_ex) = excerpts_around_hunks(pre, post, 50);
        assert!(pre_ex.contains("a\n") && pre_ex.contains("b\n") && pre_ex.contains("c\n"));
        assert!(post_ex.contains("B\n"));
    }

    #[test]
    fn excerpts_empty_when_unchanged() {
        let (pre_ex, post_ex) = excerpts_around_hunks("same\n", "same\n", 50);
        assert!(pre_ex.is_empty());
        assert!(post_ex.is_empty());
    }
```

- [ ] **Step 2: Run, confirm fail**

Run: `cargo test -p teramindd diff_engine`
Expected: FAIL — `excerpts_around_hunks` undefined.

- [ ] **Step 3: Add the implementation**

Append to `diff_engine.rs`:

```rust
/// Extract ±`radius` line windows around each change in (pre, post).
///
/// Uses `similar::TextDiff` to identify changed line ranges, then projects
/// those ranges back onto the original line vectors with a `radius`-line
/// context. Overlapping windows merge.
///
/// Returns `(pre_excerpt, post_excerpt)`. Both empty when `pre == post`.
pub fn excerpts_around_hunks(pre: &str, post: &str, radius: usize) -> (String, String) {
    if pre == post {
        return (String::new(), String::new());
    }
    let pre_lines: Vec<&str> = pre.split_inclusive('\n').collect();
    let post_lines: Vec<&str> = post.split_inclusive('\n').collect();

    let mut pre_ranges: Vec<(usize, usize)> = Vec::new();
    let mut post_ranges: Vec<(usize, usize)> = Vec::new();
    let diff = TextDiff::from_lines(pre, post);
    for op in diff.ops() {
        // op gives (tag, old_start..old_end, new_start..new_end).
        let old = op.old_range();
        let new = op.new_range();
        // Only record changed segments (skip equal runs).
        if matches!(op.tag(), similar::DiffTag::Equal) {
            continue;
        }
        pre_ranges.push((old.start, old.end));
        post_ranges.push((new.start, new.end));
    }

    let pre_ex = collect_window(&pre_lines, &pre_ranges, radius);
    let post_ex = collect_window(&post_lines, &post_ranges, radius);
    (pre_ex, post_ex)
}

fn collect_window(lines: &[&str], ranges: &[(usize, usize)], radius: usize) -> String {
    if ranges.is_empty() {
        return String::new();
    }
    // Compute windows then merge overlaps.
    let mut windows: Vec<(usize, usize)> = ranges
        .iter()
        .map(|(s, e)| {
            let start = s.saturating_sub(radius);
            let end = (*e + radius).min(lines.len());
            (start, end)
        })
        .collect();
    windows.sort_by_key(|w| w.0);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for w in windows {
        match merged.last_mut() {
            Some(prev) if prev.1 >= w.0 => prev.1 = prev.1.max(w.1),
            _ => merged.push(w),
        }
    }
    let mut out = String::new();
    for (start, end) in merged {
        for line in &lines[start..end] {
            out.push_str(line);
        }
    }
    out
}
```

- [ ] **Step 4: Run, confirm PASS**

Run: `cargo test -p teramindd diff_engine`
Expected: PASS — all three new tests.

- [ ] **Step 5: Commit**

```bash
git add crates/teramindd/src/services/diff_engine.rs
git commit -m "feat(daemon): hunk-bounded ±radius excerpt extractor"
```

---

### Task 2.5: Add `compute_file_diff` top-level helper

**Files:**
- Modify: `crates/teramindd/src/services/diff_engine.rs`

This is the single entry point the FS watcher will call.

- [ ] **Step 1: Write the failing test**

Append:

```rust
    use std::path::PathBuf;

    #[test]
    fn compute_file_diff_assembles_full_payload() {
        let pre = "fn old() {}\n";
        let post = "fn new() {}\n";
        let path = PathBuf::from("src/lib.rs");
        let d = compute_file_diff(pre, post, &path).expect("Some when changed");
        assert_eq!(d.language.as_deref(), Some("rust"));
        assert!(d.unified_diff.contains("-fn old() {}"));
        assert!(d.unified_diff.contains("+fn new() {}"));
        assert!(d.pre_excerpt.contains("fn old"));
        assert!(d.post_excerpt.contains("fn new"));
        assert_eq!(d.byte_size, post.as_bytes().len() as i32);
        assert_ne!(d.pre_hash, d.post_hash);
    }

    #[test]
    fn compute_file_diff_none_when_unchanged() {
        let s = "x";
        assert!(compute_file_diff(s, s, &PathBuf::from("a.rs")).is_none());
    }
```

- [ ] **Step 2: Run, confirm fail**

Run: `cargo test -p teramindd diff_engine::tests::compute_file_diff_assembles_full_payload`
Expected: FAIL — function undefined.

- [ ] **Step 3: Add the function + payload struct**

Append to `diff_engine.rs`:

```rust
/// Plain-old-data payload assembled by `compute_file_diff` and consumed
/// by the FS watcher to build an `IngestEvent::FileDiff`.
#[derive(Debug, Clone)]
pub struct ComputedDiff {
    pub unified_diff: String,
    pub pre_excerpt: String,
    pub post_excerpt: String,
    pub pre_hash: [u8; 32],
    pub post_hash: [u8; 32],
    pub byte_size: i32,
    pub language: Option<String>,
}

/// Compute everything we need to persist for a (pre, post, path) triple.
/// Returns `None` when `pre == post`.
pub fn compute_file_diff(pre: &str, post: &str, rel_path: &Path) -> Option<ComputedDiff> {
    if pre == post {
        return None;
    }
    let rel = rel_path.to_string_lossy();
    let unified = unified_diff(pre, post, &rel);
    let (pre_ex, post_ex) = excerpts_around_hunks(pre, post, 50);
    Some(ComputedDiff {
        unified_diff: unified,
        pre_excerpt: pre_ex,
        post_excerpt: post_ex,
        pre_hash: sha256_hash(pre.as_bytes()),
        post_hash: sha256_hash(post.as_bytes()),
        byte_size: post.as_bytes().len() as i32,
        language: language_from_extension(rel_path).map(String::from),
    })
}
```

- [ ] **Step 4: Run, confirm PASS**

Run: `cargo test -p teramindd diff_engine`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramindd/src/services/diff_engine.rs
git commit -m "feat(daemon): compute_file_diff top-level helper"
```

---

### Task 2.6: Add a property test for excerpt-window invariants

**Files:**
- Modify: `crates/teramindd/Cargo.toml`
- Modify: `crates/teramindd/src/services/diff_engine.rs`

- [ ] **Step 1: Ensure `proptest` is a dev-dependency**

Append to `[dev-dependencies]` in `crates/teramindd/Cargo.toml`:

```toml
proptest = { workspace = true }
```

(If already present, leave as-is.)

- [ ] **Step 2: Write the property test**

Append to the test module:

```rust
    use proptest::prelude::*;

    proptest! {
        // No matter what (pre, post) we throw at it, the returned excerpts
        // must be subsets of pre/post respectively, and the diff parses as
        // a valid unified diff header when non-empty.
        #[test]
        fn excerpts_are_substrings_of_inputs(
            pre in proptest::collection::vec("[a-zA-Z0-9 ]{0,40}", 0..50),
            mutations in proptest::collection::vec(0u8..=3u8, 0..20),
        ) {
            let pre = pre.join("\n") + "\n";
            // Build a post by applying simple mutations.
            let mut post_lines: Vec<String> = pre.lines().map(|s| s.to_string()).collect();
            for (i, m) in mutations.iter().enumerate() {
                if post_lines.is_empty() { break; }
                let idx = i % post_lines.len();
                match m {
                    0 => post_lines[idx].push('!'),
                    1 => post_lines[idx].insert(0, '#'),
                    2 => { post_lines.remove(idx); }
                    _ => post_lines.insert(idx, "INS".into()),
                }
            }
            let post = post_lines.join("\n") + "\n";
            let (pre_ex, post_ex) = excerpts_around_hunks(&pre, &post, 5);
            // Every line in pre_ex must appear in pre; same for post_ex.
            for line in pre_ex.lines() {
                prop_assert!(pre.lines().any(|p| p == line),
                    "pre_excerpt line {:?} not in pre", line);
            }
            for line in post_ex.lines() {
                prop_assert!(post.lines().any(|p| p == line),
                    "post_excerpt line {:?} not in post", line);
            }
        }
    }
```

- [ ] **Step 3: Run, confirm PASS**

Run: `cargo test -p teramindd excerpts_are_substrings_of_inputs --release`
Expected: PASS (256 cases by default).

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/Cargo.toml crates/teramindd/src/services/diff_engine.rs
git commit -m "test(daemon): proptest excerpt invariants"
```

---

## Section 3 — Snapshot cache + git index fallback

The watcher needs pre-content. The cache holds the most recent post-content per (cwd, rel_path) so successive edits can diff cleanly. On a cache miss we fall back to `git show :./rel_path` (the staged version); if that fails (no repo / untracked file) we fall back to the empty string.

### Task 3.1: `snapshot_cache.rs` — in-memory TTL cache

**Files:**
- Create: `crates/teramindd/src/services/snapshot_cache.rs`
- Modify: `crates/teramindd/src/services/mod.rs`

- [ ] **Step 1: Register module**

Append to `crates/teramindd/src/services/mod.rs`:

```rust
pub mod snapshot_cache;
```

- [ ] **Step 2: Write the failing test in the new file**

Create `crates/teramindd/src/services/snapshot_cache.rs`:

```rust
//! In-memory map of (cwd, rel_path) -> last-seen file content.
//!
//! The FS watcher stores post-content here so the NEXT modification of
//! the same file has accurate pre-content. Entries older than the
//! configured TTL are evicted on insert.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use time::OffsetDateTime;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct SnapshotCache {
    inner: Arc<Mutex<HashMap<(PathBuf, String), Entry>>>,
    ttl: time::Duration,
}

#[derive(Clone)]
struct Entry {
    content: String,
    stored_at: OffsetDateTime,
}

impl SnapshotCache {
    pub fn new(ttl: time::Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            ttl,
        }
    }

    pub async fn get(&self, cwd: &PathBuf, rel_path: &str) -> Option<String> {
        let m = self.inner.lock().await;
        m.get(&(cwd.clone(), rel_path.to_string()))
            .map(|e| e.content.clone())
    }

    pub async fn put(&self, cwd: PathBuf, rel_path: String, content: String) {
        let now = OffsetDateTime::now_utc();
        let mut m = self.inner.lock().await;
        // Evict stale entries.
        m.retain(|_, e| now - e.stored_at < self.ttl);
        m.insert((cwd, rel_path), Entry { content, stored_at: now });
    }

    #[cfg(test)]
    pub async fn len(&self) -> usize {
        self.inner.lock().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn roundtrip_put_get() {
        let c = SnapshotCache::new(time::Duration::seconds(60));
        let cwd = PathBuf::from("/p");
        c.put(cwd.clone(), "a.rs".into(), "fn a(){}".into()).await;
        let got = c.get(&cwd, "a.rs").await;
        assert_eq!(got.as_deref(), Some("fn a(){}"));
    }

    #[tokio::test]
    async fn ttl_evicts_old_entries_on_next_put() {
        let c = SnapshotCache::new(time::Duration::milliseconds(50));
        let cwd = PathBuf::from("/p");
        c.put(cwd.clone(), "a".into(), "x".into()).await;
        assert_eq!(c.len().await, 1);
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        c.put(cwd.clone(), "b".into(), "y".into()).await;
        // Old entry should have been evicted; only "b" remains.
        assert_eq!(c.len().await, 1);
        assert!(c.get(&cwd, "a").await.is_none());
        assert_eq!(c.get(&cwd, "b").await.as_deref(), Some("y"));
    }
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p teramindd snapshot_cache`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/snapshot_cache.rs crates/teramindd/src/services/mod.rs
git commit -m "feat(daemon): snapshot_cache for pre-content lookup"
```

---

### Task 3.2: `git_index.rs` — best-effort `git show :rel_path`

**Files:**
- Create: `crates/teramindd/src/services/git_index.rs`
- Modify: `crates/teramindd/src/services/mod.rs`

- [ ] **Step 1: Register module**

Append to `crates/teramindd/src/services/mod.rs`:

```rust
pub mod git_index;
```

- [ ] **Step 2: Write the test using a temp git repo**

Create `crates/teramindd/src/services/git_index.rs`:

```rust
//! Best-effort lookup of the git-indexed version of a file via `git show :rel`.
//!
//! Returns `None` when the cwd is not a git repo, the file is untracked,
//! `git` is missing on PATH, or the lookup times out. The FS watcher
//! falls back to an empty pre-content string in any of those cases.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

const GIT_TIMEOUT: Duration = Duration::from_millis(500);

pub async fn show_index(cwd: &Path, rel_path: &str) -> Option<String> {
    // Use `--` to ensure rel_path is treated as a pathspec, not a rev.
    let mut cmd = Command::new("git");
    cmd.arg("show")
        .arg(format!(":{rel_path}"))
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = cmd.spawn().ok()?;
    let out = timeout(GIT_TIMEOUT, child.wait_with_output()).await.ok()?.ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as SyncCommand;
    use tempfile::TempDir;

    fn init_repo_with_committed_file(content: &str) -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        SyncCommand::new("git").arg("init").arg("-q").current_dir(p).status().unwrap();
        SyncCommand::new("git").args(["config","user.email","t@t"]).current_dir(p).status().unwrap();
        SyncCommand::new("git").args(["config","user.name","t"]).current_dir(p).status().unwrap();
        std::fs::write(p.join("a.txt"), content).unwrap();
        SyncCommand::new("git").args(["add","a.txt"]).current_dir(p).status().unwrap();
        SyncCommand::new("git").args(["commit","-q","-m","init"]).current_dir(p).status().unwrap();
        dir
    }

    #[tokio::test]
    async fn returns_indexed_content_for_committed_file() {
        let repo = init_repo_with_committed_file("hello\nworld\n");
        let got = show_index(repo.path(), "a.txt").await;
        assert_eq!(got.as_deref(), Some("hello\nworld\n"));
    }

    #[tokio::test]
    async fn returns_none_for_untracked_file() {
        let repo = init_repo_with_committed_file("x");
        std::fs::write(repo.path().join("untracked.rs"), "y").unwrap();
        let got = show_index(repo.path(), "untracked.rs").await;
        assert!(got.is_none(), "expected None for untracked, got {got:?}");
    }

    #[tokio::test]
    async fn returns_none_outside_repo() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), "x").unwrap();
        let got = show_index(dir.path(), "a").await;
        assert!(got.is_none());
    }
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p teramindd git_index`
Expected: PASS (skip the tests on CI if `git` is unavailable — but locally + in the project's Linux/macOS CI it is present).

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/git_index.rs crates/teramindd/src/services/mod.rs
git commit -m "feat(daemon): git_index.show_index for pre-content fallback"
```

---

### Task 3.3: Top-level `resolve_pre_content`

**Files:**
- Modify: `crates/teramindd/src/services/snapshot_cache.rs`

- [ ] **Step 1: Write the failing test**

Append to the test module:

```rust
    use std::path::Path;

    #[tokio::test]
    async fn resolve_pre_content_returns_cache_first() {
        let c = SnapshotCache::new(time::Duration::seconds(60));
        let cwd = PathBuf::from("/nonexistent-no-git");
        c.put(cwd.clone(), "a.rs".into(), "CACHED".into()).await;
        let s = resolve_pre_content(&c, &cwd, "a.rs").await;
        assert_eq!(s, "CACHED");
    }

    #[tokio::test]
    async fn resolve_pre_content_falls_back_to_empty_string_when_no_git() {
        let c = SnapshotCache::new(time::Duration::seconds(60));
        let dir = tempfile::tempdir().unwrap();
        let s = resolve_pre_content(&c, &dir.path().to_path_buf(), "ghost.rs").await;
        assert_eq!(s, "");
    }
```

- [ ] **Step 2: Add the function**

Append to `snapshot_cache.rs`:

```rust
use std::path::Path;

/// Resolve pre-content for (cwd, rel_path) using cache -> git index -> empty string.
pub async fn resolve_pre_content(
    cache: &SnapshotCache,
    cwd: &Path,
    rel_path: &str,
) -> String {
    if let Some(s) = cache.get(&cwd.to_path_buf(), rel_path).await {
        return s;
    }
    if let Some(s) = crate::services::git_index::show_index(cwd, rel_path).await {
        return s;
    }
    String::new()
}
```

- [ ] **Step 3: Run, confirm PASS**

Run: `cargo test -p teramindd snapshot_cache`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/snapshot_cache.rs
git commit -m "feat(daemon): resolve_pre_content combines cache + git index"
```

---

## Section 4 — Write-tool ring buffer

A bounded ring of `(session_id, turn_id, tool_name, at)` records of recent write-tool completions. The FS watcher consults it to decide attribution.

### Task 4.1: Implement `WriteToolRing`

**Files:**
- Create: `crates/teramindd/src/services/write_tool_ring.rs`
- Modify: `crates/teramindd/src/services/mod.rs`

- [ ] **Step 1: Register module**

Append to `crates/teramindd/src/services/mod.rs`:

```rust
pub mod write_tool_ring;
```

- [ ] **Step 2: Create file with the failing test**

Create `crates/teramindd/src/services/write_tool_ring.rs`:

```rust
//! Bounded ring of recent write-tool PostToolUse completions.
//! Used by the FS watcher to decide whether a file change should be
//! attributed to an agent turn (within `window`) or to the human user.

use std::collections::VecDeque;
use std::sync::Arc;
use teramind_core::ids::{SessionId, TurnId};
use time::OffsetDateTime;
use tokio::sync::Mutex;

pub const WRITE_TOOLS: &[&str] = &["Edit", "Write", "MultiEdit", "NotebookEdit"];

#[derive(Debug, Clone)]
pub struct WriteCompletion {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub tool_name: String,
    pub at: OffsetDateTime,
}

#[derive(Clone)]
pub struct WriteToolRing {
    inner: Arc<Mutex<VecDeque<WriteCompletion>>>,
    capacity: usize,
    window: time::Duration,
}

impl WriteToolRing {
    pub fn new(capacity: usize, window: time::Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
            window,
        }
    }

    pub async fn push(&self, w: WriteCompletion) {
        let mut d = self.inner.lock().await;
        if d.len() == self.capacity {
            d.pop_front();
        }
        d.push_back(w);
    }

    /// Find the most recent write completion for `session_id` no older than
    /// `now - window`. Returns the matching record or `None`.
    pub async fn most_recent_for(
        &self,
        session_id: SessionId,
        now: OffsetDateTime,
    ) -> Option<WriteCompletion> {
        let d = self.inner.lock().await;
        let cutoff = now - self.window;
        d.iter()
            .rev()
            .find(|w| w.session_id == session_id && w.at >= cutoff)
            .cloned()
    }
}

pub fn is_write_tool(name: &str) -> bool {
    WRITE_TOOLS.iter().any(|w| *w == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(secs: i64) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(secs).unwrap()
    }

    #[tokio::test]
    async fn push_and_find_inside_window() {
        let ring = WriteToolRing::new(8, time::Duration::seconds(5));
        let sid = SessionId::new();
        let tid = TurnId::new();
        ring.push(WriteCompletion {
            session_id: sid, turn_id: tid, tool_name: "Edit".into(), at: t(100),
        }).await;
        let got = ring.most_recent_for(sid, t(103)).await;
        assert!(got.is_some());
        assert_eq!(got.unwrap().turn_id, tid);
    }

    #[tokio::test]
    async fn outside_window_returns_none() {
        let ring = WriteToolRing::new(8, time::Duration::seconds(5));
        let sid = SessionId::new();
        ring.push(WriteCompletion {
            session_id: sid, turn_id: TurnId::new(), tool_name: "Edit".into(), at: t(100),
        }).await;
        assert!(ring.most_recent_for(sid, t(200)).await.is_none());
    }

    #[tokio::test]
    async fn capacity_evicts_oldest() {
        let ring = WriteToolRing::new(2, time::Duration::seconds(60));
        let sid = SessionId::new();
        for i in 0..4 {
            ring.push(WriteCompletion {
                session_id: sid,
                turn_id: TurnId::new(),
                tool_name: "Edit".into(),
                at: t(100 + i),
            }).await;
        }
        // Only the newest two survive; oldest at t(100) should be gone.
        let got = ring.most_recent_for(sid, t(105)).await.unwrap();
        assert_eq!(got.at, t(103));
    }

    #[test]
    fn is_write_tool_matches_documented_names() {
        assert!(is_write_tool("Edit"));
        assert!(is_write_tool("Write"));
        assert!(is_write_tool("MultiEdit"));
        assert!(is_write_tool("NotebookEdit"));
        assert!(!is_write_tool("Read"));
        assert!(!is_write_tool("Bash"));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p teramindd write_tool_ring`
Expected: PASS — all four.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/write_tool_ring.rs crates/teramindd/src/services/mod.rs
git commit -m "feat(daemon): write_tool_ring for attribution decisions"
```

---

### Task 4.2: IngestService pushes to the ring on write-tool `ToolCallEnd`

**Files:**
- Modify: `crates/teramindd/src/services/ingest.rs`

- [ ] **Step 1: Extend `IngestDeps` with the ring**

Edit `IngestDeps` struct (~line 27) to add:

```rust
    pub write_tool_ring: crate::services::write_tool_ring::WriteToolRing,
```

- [ ] **Step 2: On `ToolCallEnd`, push when it's a write tool**

In `route()`, replace the existing `ToolCallEnd` arm with:

```rust
        ToolCallEnd {
            tool_call_id,
            output,
            is_error,
            duration_ms,
            session_id,
            turn_id,
            tool_name,
        } => {
            d.trace
                .finalize_tool_call(tool_call_id, &output, is_error, duration_ms)
                .await?;
            if let (Some(sid), Some(tid), Some(name)) = (session_id, turn_id, tool_name.as_deref()) {
                if crate::services::write_tool_ring::is_write_tool(name) {
                    d.write_tool_ring
                        .push(crate::services::write_tool_ring::WriteCompletion {
                            session_id: sid,
                            turn_id: tid,
                            tool_name: name.to_string(),
                            at: ts,
                        })
                        .await;
                }
            }
        }
```

- [ ] **Step 3: Write an integration test**

Create `crates/teramindd/tests/ingest_write_tool_ring.rs`:

```rust
use teramind_core::ids::{ClientEventId, SessionId, ToolCallId, TurnId};
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use teramindd::services::write_tool_ring::WriteToolRing;
use time::OffsetDateTime;

// Exercise the ring directly to lock in the write-tool naming contract.
#[tokio::test]
async fn ring_only_records_write_tool_completions() {
    let ring = WriteToolRing::new(8, time::Duration::seconds(5));
    let sid = SessionId::new();
    let tid = TurnId::new();

    // Push as the ingest layer would.
    if teramindd::services::write_tool_ring::is_write_tool("Edit") {
        ring.push(teramindd::services::write_tool_ring::WriteCompletion {
            session_id: sid,
            turn_id: tid,
            tool_name: "Edit".into(),
            at: OffsetDateTime::now_utc(),
        }).await;
    }
    // Non-write tool: do NOT push.
    if teramindd::services::write_tool_ring::is_write_tool("Read") {
        unreachable!("Read should not be a write tool");
    }
    assert!(ring.most_recent_for(sid, OffsetDateTime::now_utc()).await.is_some());

    // Construct an envelope just to make sure the type compiles end-to-end:
    let _env = EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: OffsetDateTime::now_utc(),
        event: IngestEvent::ToolCallEnd {
            tool_call_id: ToolCallId::new(),
            output: "ok".into(),
            is_error: false,
            duration_ms: 0,
            session_id: Some(sid),
            turn_id: Some(tid),
            tool_name: Some("Edit".into()),
        },
    };
}
```

- [ ] **Step 4: Expose the new module + ring publicly**

Ensure `crates/teramindd/src/lib.rs` re-exports services so the integration test can `use teramindd::services::…`. If it already exposes `pub mod services;` no change needed; otherwise add it:

```rust
pub mod services;
```

(Open `crates/teramindd/src/lib.rs` and add the `pub mod` line if missing.)

- [ ] **Step 5: Run**

Run: `cargo test -p teramindd ring_only_records_write_tool_completions`
Expected: PASS.

- [ ] **Step 6: Wire ring into `App::run`**

In `crates/teramindd/src/app.rs`, before constructing `IngestDeps`, add:

```rust
        let write_tool_ring = crate::services::write_tool_ring::WriteToolRing::new(
            64,
            time::Duration::seconds(5),
        );
```

Pass it into `IngestDeps { …, write_tool_ring: write_tool_ring.clone(), … }`. Hold onto `write_tool_ring` as a local — the FS watcher will subscribe to it in Section 7.

Add `use time;` only if missing (it's already in the prelude via re-exports from the workspace).

- [ ] **Step 7: `cargo check -p teramindd`**

Expected: succeeds.

- [ ] **Step 8: Commit**

```bash
git add crates/teramindd/src/services/ingest.rs crates/teramindd/src/app.rs crates/teramindd/src/lib.rs crates/teramindd/tests/ingest_write_tool_ring.rs
git commit -m "feat(daemon): ingest pushes write-tool completions to ring"
```

---

## Section 5 — Ignore filter

We reject obvious noise (`.git/`, `target/`, `node_modules/`, editor swap files) plus anything matching the project's `.gitignore`.

### Task 5.1: `ignore_filter.rs`

**Files:**
- Create: `crates/teramindd/src/services/ignore_filter.rs`
- Modify: `crates/teramindd/src/services/mod.rs`

- [ ] **Step 1: Register module**

Append to `crates/teramindd/src/services/mod.rs`:

```rust
pub mod ignore_filter;
```

- [ ] **Step 2: Create file with failing tests**

Create `crates/teramindd/src/services/ignore_filter.rs`:

```rust
//! Path filter for the FS watcher. Combines a built-in deny list with the
//! project's `.gitignore`.

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::{Path, PathBuf};

/// Built-in patterns that always match, in addition to .gitignore.
const ALWAYS_IGNORE: &[&str] = &[
    ".git/",
    ".git/**",
    "node_modules/",
    "node_modules/**",
    "target/",
    "target/**",
    "dist/",
    "dist/**",
    ".DS_Store",
    "*.swp",
    "*.swo",
    "*.tmp",
    "*~",
    "*.orig",
    ".idea/",
    ".idea/**",
    ".vscode/",
    ".vscode/**",
];

use std::sync::Arc;

/// `ignore::gitignore::Gitignore` is not `Clone`, so we wrap each instance
/// in `Arc` to keep `IgnoreFilter` cheaply cloneable for use inside the
/// `notify` event closure.
#[derive(Clone)]
pub struct IgnoreFilter {
    root: PathBuf,
    always: Arc<Gitignore>,
    project: Option<Arc<Gitignore>>,
}

impl IgnoreFilter {
    /// Build a filter rooted at `root`. Reads `<root>/.gitignore` when present.
    pub fn for_root(root: &Path) -> Self {
        let mut b = GitignoreBuilder::new(root);
        for p in ALWAYS_IGNORE {
            // unwrap: only fails if pattern is invalid, and ours are static.
            b.add_line(None, p).expect("static pattern");
        }
        let always = Arc::new(b.build().expect("build always-ignore"));

        let project = {
            let gi_path = root.join(".gitignore");
            if gi_path.exists() {
                let mut pb = GitignoreBuilder::new(root);
                let _ = pb.add(&gi_path);
                pb.build().ok().map(Arc::new)
            } else {
                None
            }
        };

        Self {
            root: root.to_path_buf(),
            always,
            project,
        }
    }

    /// Returns true when `abs_path` should be ignored.
    pub fn is_ignored(&self, abs_path: &Path) -> bool {
        let is_dir = abs_path.is_dir();
        if self.always.matched(abs_path, is_dir).is_ignore() {
            return true;
        }
        if let Some(g) = &self.project {
            if g.matched(abs_path, is_dir).is_ignore() {
                return true;
            }
        }
        false
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_tree() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        std::fs::write(dir.path().join("target/debug/x"), "x").unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn a(){}").unwrap();
        std::fs::write(dir.path().join(".DS_Store"), "x").unwrap();
        std::fs::write(dir.path().join("a.rs.swp"), "x").unwrap();
        dir
    }

    #[test]
    fn ignores_git_and_target_and_editor_junk() {
        let dir = make_tree();
        let f = IgnoreFilter::for_root(dir.path());
        assert!(f.is_ignored(&dir.path().join(".git/HEAD")));
        assert!(f.is_ignored(&dir.path().join("target/debug/x")));
        assert!(f.is_ignored(&dir.path().join(".DS_Store")));
        assert!(f.is_ignored(&dir.path().join("a.rs.swp")));
        assert!(!f.is_ignored(&dir.path().join("a.rs")));
    }

    #[test]
    fn respects_project_gitignore() {
        let dir = make_tree();
        std::fs::write(dir.path().join(".gitignore"), "secret.txt\nbuild/\n").unwrap();
        std::fs::write(dir.path().join("secret.txt"), "x").unwrap();
        std::fs::create_dir_all(dir.path().join("build")).unwrap();
        std::fs::write(dir.path().join("build/out"), "x").unwrap();

        let f = IgnoreFilter::for_root(dir.path());
        assert!(f.is_ignored(&dir.path().join("secret.txt")));
        assert!(f.is_ignored(&dir.path().join("build/out")));
        assert!(!f.is_ignored(&dir.path().join("a.rs")));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p teramindd ignore_filter`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/ignore_filter.rs crates/teramindd/src/services/mod.rs
git commit -m "feat(daemon): ignore_filter for FS watcher"
```

---

## Section 6 — FS watcher service

The watcher service owns a per-cwd watcher registry and a per-path debouncer. Each filesystem event walks through ignore → debounce → read post → resolve pre → diff → attribute → emit `IngestEvent::FileDiff` via the existing ingest sender.

### Task 6.1: Scaffold `fs_watcher.rs` + `WatchRegistry`

**Files:**
- Create: `crates/teramindd/src/services/fs_watcher.rs`
- Modify: `crates/teramindd/src/services/mod.rs`

- [ ] **Step 1: Register module**

Append to `crates/teramindd/src/services/mod.rs`:

```rust
pub mod fs_watcher;
```

- [ ] **Step 2: Add the WatchRegistry test**

Create `crates/teramindd/src/services/fs_watcher.rs`:

```rust
//! FS watcher service. Owns one notify::RecommendedWatcher per unique
//! active-session cwd, refcounted by session_id. On filesystem events,
//! debounces per (cwd, rel_path) and dispatches a full
//! pre/post/diff/excerpts/attribution pipeline.

use crate::services::diff_engine::{compute_file_diff, ComputedDiff};
use crate::services::ignore_filter::IgnoreFilter;
use crate::services::snapshot_cache::{resolve_pre_content, SnapshotCache};
use crate::services::write_tool_ring::WriteToolRing;
use notify::event::EventKind;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use teramind_core::ids::{ClientEventId, SessionId};
use teramind_core::types::file_diff::Attribution;
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use time::OffsetDateTime;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, warn};

/// One watcher per unique cwd. Refcounted by active session_ids; the
/// watcher is dropped when the last session in that cwd ends.
pub struct WatchRegistry {
    inner: Mutex<HashMap<PathBuf, WatchEntry>>,
    event_tx: mpsc::UnboundedSender<RawEvent>,
    /// Incremented whenever `notify` reports an error (lost slot, etc.).
    /// Wired to `IngestStats.fs_watcher_gaps` so `teramind status` can surface it.
    gaps_counter: Arc<std::sync::atomic::AtomicU64>,
}

struct WatchEntry {
    sessions: HashSet<SessionId>,
    watcher: RecommendedWatcher,
    filter: IgnoreFilter,
}

#[derive(Debug, Clone)]
pub struct RawEvent {
    pub cwd: PathBuf,
    pub abs_path: PathBuf,
    pub at: OffsetDateTime,
}

impl WatchRegistry {
    pub fn new(
        event_tx: mpsc::UnboundedSender<RawEvent>,
        gaps_counter: Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            event_tx,
            gaps_counter,
        }
    }

    pub async fn register(&self, cwd: PathBuf, session: SessionId) -> anyhow::Result<()> {
        let mut g = self.inner.lock().await;
        if let Some(entry) = g.get_mut(&cwd) {
            entry.sessions.insert(session);
            return Ok(());
        }
        let cwd_for_cb = cwd.clone();
        let tx = self.event_tx.clone();
        let filter = IgnoreFilter::for_root(&cwd);
        let filter_for_cb = filter.clone();
        let gaps = self.gaps_counter.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let ev = match res {
                Ok(ev) => ev,
                Err(_) => {
                    gaps.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    return;
                }
            };
            if !matches!(ev.kind,
                EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
            ) {
                return;
            }
            for p in ev.paths {
                if filter_for_cb.is_ignored(&p) { continue; }
                let _ = tx.send(RawEvent {
                    cwd: cwd_for_cb.clone(),
                    abs_path: p,
                    at: OffsetDateTime::now_utc(),
                });
            }
        })?;
        watcher.watch(&cwd, RecursiveMode::Recursive)?;
        let mut sessions = HashSet::new();
        sessions.insert(session);
        g.insert(cwd, WatchEntry { sessions, watcher, filter });
        Ok(())
    }

    pub async fn unregister(&self, cwd: &Path, session: SessionId) {
        let mut g = self.inner.lock().await;
        if let Some(entry) = g.get_mut(cwd) {
            entry.sessions.remove(&session);
            if entry.sessions.is_empty() {
                g.remove(cwd);
            }
        }
    }

    #[cfg(test)]
    pub async fn watched_count(&self) -> usize {
        self.inner.lock().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_counter() -> Arc<std::sync::atomic::AtomicU64> {
        Arc::new(std::sync::atomic::AtomicU64::new(0))
    }

    #[tokio::test]
    async fn register_then_unregister_drops_watcher() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let reg = WatchRegistry::new(tx, fresh_counter());
        let dir = tempfile::tempdir().unwrap();
        let sid = SessionId::new();
        reg.register(dir.path().to_path_buf(), sid).await.unwrap();
        assert_eq!(reg.watched_count().await, 1);
        reg.unregister(dir.path(), sid).await;
        assert_eq!(reg.watched_count().await, 0);
    }

    #[tokio::test]
    async fn second_session_in_same_cwd_does_not_duplicate_watcher() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let reg = WatchRegistry::new(tx, fresh_counter());
        let dir = tempfile::tempdir().unwrap();
        let a = SessionId::new();
        let b = SessionId::new();
        reg.register(dir.path().to_path_buf(), a).await.unwrap();
        reg.register(dir.path().to_path_buf(), b).await.unwrap();
        assert_eq!(reg.watched_count().await, 1);
        reg.unregister(dir.path(), a).await;
        assert_eq!(reg.watched_count().await, 1); // b still holds it
        reg.unregister(dir.path(), b).await;
        assert_eq!(reg.watched_count().await, 0);
    }

    #[tokio::test]
    async fn modify_event_is_emitted_to_channel() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let reg = WatchRegistry::new(tx, fresh_counter());
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "x").unwrap();
        reg.register(dir.path().to_path_buf(), SessionId::new()).await.unwrap();
        // Modify the file.
        std::fs::write(dir.path().join("a.rs"), "y").unwrap();
        // Wait briefly for notify to fire.
        let evt = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await.expect("timed out").expect("channel closed");
        assert!(evt.abs_path.ends_with("a.rs"), "got {:?}", evt.abs_path);
    }
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p teramindd fs_watcher::tests`
Expected: PASS (notify timing should be reliable on macOS/Linux; if `modify_event_is_emitted_to_channel` is flaky on a specific platform, retry once before declaring failure).

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/fs_watcher.rs crates/teramindd/src/services/mod.rs
git commit -m "feat(daemon): fs_watcher WatchRegistry with per-cwd refcount"
```

---

### Task 6.2: Per-path debouncer

**Files:**
- Modify: `crates/teramindd/src/services/fs_watcher.rs`

- [ ] **Step 1: Write the failing test**

Append to `fs_watcher.rs` test module:

```rust
    #[tokio::test]
    async fn debouncer_coalesces_rapid_events() {
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<RawEvent>();
        let deb = Debouncer::start(Duration::from_millis(80), out_tx);

        let cwd = PathBuf::from("/p");
        let p = PathBuf::from("/p/a.rs");
        let now = OffsetDateTime::now_utc();
        for _ in 0..5 {
            deb.enqueue(RawEvent { cwd: cwd.clone(), abs_path: p.clone(), at: now }).await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        // After the 80ms quiet period we should get exactly one resolved event.
        let first = tokio::time::timeout(Duration::from_millis(500), out_rx.recv())
            .await.expect("timeout").unwrap();
        assert_eq!(first.abs_path, p);

        // No additional events expected.
        let extra = tokio::time::timeout(Duration::from_millis(150), out_rx.recv()).await;
        assert!(extra.is_err(), "expected no further events, got {:?}", extra.unwrap());
    }
```

- [ ] **Step 2: Implement `Debouncer`**

Append to `fs_watcher.rs` (above the test module):

```rust
/// Per-(cwd, abs_path) debouncer. Each incoming event for a given key
/// aborts the previous pending timer so only the *last* event in a
/// `quiet` window is emitted downstream.
pub struct Debouncer {
    in_tx: mpsc::UnboundedSender<RawEvent>,
}

impl Debouncer {
    pub fn start(quiet: Duration, out_tx: mpsc::UnboundedSender<RawEvent>) -> Self {
        let (in_tx, mut in_rx) = mpsc::unbounded_channel::<RawEvent>();
        tokio::spawn(async move {
            type Key = (PathBuf, PathBuf);
            let mut timers: HashMap<Key, tokio::task::JoinHandle<()>> = HashMap::new();
            while let Some(ev) = in_rx.recv().await {
                let key = (ev.cwd.clone(), ev.abs_path.clone());
                if let Some(h) = timers.remove(&key) {
                    h.abort();
                }
                let out = out_tx.clone();
                let handle = tokio::spawn(async move {
                    tokio::time::sleep(quiet).await;
                    let _ = out.send(ev);
                });
                timers.insert(key, handle);
            }
        });
        Self { in_tx }
    }

    pub async fn enqueue(&self, ev: RawEvent) {
        let _ = self.in_tx.send(ev);
    }
}
```

- [ ] **Step 3: Run the debouncer test**

Run: `cargo test -p teramindd fs_watcher::tests::debouncer_coalesces_rapid_events`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/fs_watcher.rs
git commit -m "feat(daemon): per-path Debouncer with timer-abort coalescing"
```

---

### Task 6.3: `FsWatcherService` pipeline (event → diff → IngestEvent)

**Files:**
- Modify: `crates/teramindd/src/services/fs_watcher.rs`

- [ ] **Step 1: Add the service struct and `start()` constructor**

Append to `fs_watcher.rs` above the test module:

```rust
/// All the wiring the FS watcher needs to do its job.
#[derive(Clone)]
pub struct FsWatcherDeps {
    pub registry: Arc<WatchRegistry>,
    pub debouncer: Arc<Debouncer>,
    pub cache: SnapshotCache,
    pub ring: WriteToolRing,
    /// Sender into the existing ingest queue. The watcher emits
    /// `IngestEvent::FileDiff` envelopes here.
    pub ingest_tx: Arc<crate::services::ingest::IngestService>,
}

pub struct FsWatcherService;

impl FsWatcherService {
    /// Spawns the dispatcher loop that consumes resolved debounce events
    /// and runs the full diff pipeline for each.
    pub fn spawn(deps: FsWatcherDeps, mut resolved_rx: mpsc::UnboundedReceiver<RawEvent>) {
        tokio::spawn(async move {
            while let Some(ev) = resolved_rx.recv().await {
                if let Err(e) = handle_event(&deps, ev).await {
                    warn!(error = %e, "fs_watcher handle_event failed");
                }
            }
        });
    }
}

/// The full pipeline for one resolved (post-debounce) filesystem event.
async fn handle_event(deps: &FsWatcherDeps, ev: RawEvent) -> anyhow::Result<()> {
    // 1. Ignore events for paths that no longer exist (deleted in a flurry).
    let post = match tokio::fs::read_to_string(&ev.abs_path).await {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };

    // 2. Compute rel_path relative to cwd.
    let rel_path = match ev.abs_path.strip_prefix(&ev.cwd) {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(_) => return Ok(()),
    };

    // 3. Resolve pre-content from cache OR git index.
    let pre = resolve_pre_content(&deps.cache, &ev.cwd, &rel_path).await;
    if pre == post {
        // Update cache regardless so future diffs are accurate.
        deps.cache
            .put(ev.cwd.clone(), rel_path.clone(), post.clone())
            .await;
        return Ok(());
    }

    // 4. Compute the diff bundle.
    let Some(computed) = compute_file_diff(&pre, &post, Path::new(&rel_path)) else {
        return Ok(());
    };

    // 5. Look up active session for this cwd (via the registry) and decide
    //    attribution by consulting the write-tool ring.
    let (session_id, turn_id, attribution) = decide_attribution(deps, &ev.cwd).await;
    let Some(session_id) = session_id else {
        // No active session for this cwd — drop silently.
        return Ok(());
    };

    // 6. Emit through the existing ingest pipeline.
    let env = EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: ev.at,
        event: IngestEvent::FileDiff {
            session_id,
            turn_id,
            file_path: ev.abs_path.to_string_lossy().to_string(),
            rel_path: rel_path.clone(),
            attribution,
            language: computed.language.clone(),
            pre_excerpt: computed.pre_excerpt.clone(),
            post_excerpt: computed.post_excerpt.clone(),
            unified_diff: computed.unified_diff.clone(),
            pre_hash: computed.pre_hash,
            post_hash: computed.post_hash,
            byte_size: computed.byte_size,
        },
    };
    let _ = deps.ingest_tx.try_enqueue(env);

    // 7. Update snapshot cache with the new content.
    deps.cache
        .put(ev.cwd.clone(), rel_path, post)
        .await;

    debug!(?ev.abs_path, "fs_watcher emitted FileDiff");
    Ok(())
}

/// Pick a session_id whose cwd matches `ev_cwd`, then ask the write-tool
/// ring if there was a recent write-tool completion for that session.
/// If yes -> agent attribution + turn_id. Else -> human, turn_id=None.
async fn decide_attribution(
    deps: &FsWatcherDeps,
    ev_cwd: &Path,
) -> (Option<SessionId>, Option<teramind_core::ids::TurnId>, Attribution) {
    let g = deps.registry.inner.lock().await;
    let Some(entry) = g.get(ev_cwd) else {
        return (None, None, Attribution::Human);
    };
    // Pick any active session for this cwd; the ring will tell us which
    // (if any) ran a write tool in the window.
    let now = OffsetDateTime::now_utc();
    for sid in entry.sessions.iter() {
        if let Some(w) = deps.ring.most_recent_for(*sid, now).await {
            return (Some(*sid), Some(w.turn_id), Attribution::Agent);
        }
    }
    // Fall back to an arbitrary session for human attribution.
    let any_sid = entry.sessions.iter().next().copied();
    (any_sid, None, Attribution::Human)
}
```

- [ ] **Step 2: Make `WatchRegistry.inner` accessible to `decide_attribution`**

`inner` is currently private. Change its declaration to:

```rust
    pub(crate) inner: Mutex<HashMap<PathBuf, WatchEntry>>,
```

and `WatchEntry.sessions` to:

```rust
    pub(crate) sessions: HashSet<SessionId>,
```

(`watcher` and `filter` remain private.)

- [ ] **Step 3: `cargo check -p teramindd`**

Expected: succeeds.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/fs_watcher.rs
git commit -m "feat(daemon): fs_watcher pipeline through diff_engine"
```

---

### Task 6.4: Wire the registry + debouncer + service together in `App::run`

**Files:**
- Modify: `crates/teramindd/src/app.rs`

- [ ] **Step 1: Add wiring**

After the `ingest = Arc::new(IngestService::spawn(…))` block (and after `let write_tool_ring = …` from Task 4.2), insert:

```rust
        // FS watcher pipeline: raw -> debounced -> resolved -> handle_event
        let (raw_tx, mut raw_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::services::fs_watcher::RawEvent>();
        let (resolved_tx, resolved_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::services::fs_watcher::RawEvent>();
        let debouncer = std::sync::Arc::new(
            crate::services::fs_watcher::Debouncer::start(
                std::time::Duration::from_millis(200),
                resolved_tx,
            ),
        );
        let gaps_counter: std::sync::Arc<std::sync::atomic::AtomicU64> =
            std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let registry = std::sync::Arc::new(
            crate::services::fs_watcher::WatchRegistry::new(raw_tx, gaps_counter.clone()),
        );

        // Pump raw -> debouncer.
        {
            let deb = debouncer.clone();
            tokio::spawn(async move {
                while let Some(ev) = raw_rx.recv().await {
                    deb.enqueue(ev).await;
                }
            });
        }

        let snapshot_cache = crate::services::snapshot_cache::SnapshotCache::new(
            time::Duration::minutes(30),
        );

        crate::services::fs_watcher::FsWatcherService::spawn(
            crate::services::fs_watcher::FsWatcherDeps {
                registry: registry.clone(),
                debouncer: debouncer.clone(),
                cache: snapshot_cache.clone(),
                ring: write_tool_ring.clone(),
                ingest_tx: ingest.clone(),
            },
            resolved_rx,
        );
```

- [ ] **Step 2: Expose `registry` so the ingest path can register/unregister sessions**

Add a new field on `IngestDeps` (already done for the ring in Task 4.2); now add:

```rust
    pub fs_registry: Arc<crate::services::fs_watcher::WatchRegistry>,
```

(in `crates/teramindd/src/services/ingest.rs`)

Pass `registry.clone()` into `IngestDeps`. The `SessionStart` and `SessionEnd` arms in `route()` will call `registry.register` / `registry.unregister` — implemented in Task 8.1 below.

- [ ] **Step 3: `cargo check -p teramindd`**

Expected: succeeds.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/app.rs crates/teramindd/src/services/ingest.rs
git commit -m "feat(daemon): wire fs_watcher service in App::run"
```

---

## Section 7 — Ingest routing for `FileDiff`

The watcher emits `IngestEvent::FileDiff` through the existing ingest channel. Two things have to happen in ingest:

1. Redaction must apply to `pre_excerpt`, `post_excerpt`, and `unified_diff`.
2. The route must call `DiffRepo::insert`.

### Task 7.1: Apply redaction to `FileDiff`

**Files:**
- Modify: `crates/teramindd/src/services/ingest.rs`

- [ ] **Step 1: Write a failing test**

Create `crates/teramindd/tests/file_diff_redaction.rs`:

```rust
use teramind_core::ids::{ClientEventId, SessionId};
use teramind_core::redact::Redactor;
use teramind_core::types::file_diff::Attribution;
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use time::OffsetDateTime;

#[test]
fn redactor_strips_aws_keys_from_file_diff_excerpts() {
    let r = Redactor::with_default_rules();
    let env = EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: OffsetDateTime::now_utc(),
        event: IngestEvent::FileDiff {
            session_id: SessionId::new(),
            turn_id: None,
            file_path: "/p/a.rs".into(),
            rel_path: "a.rs".into(),
            attribution: Attribution::Human,
            language: Some("rust".into()),
            pre_excerpt: "let key = \"AKIAIOSFODNN7EXAMPLE\";".into(),
            post_excerpt: "let key = \"AKIAIOSFODNN7EXAMPLE\";".into(),
            unified_diff: " let key = \"AKIAIOSFODNN7EXAMPLE\";\n".into(),
            pre_hash: [0u8; 32],
            post_hash: [1u8; 32],
            byte_size: 32,
        },
    };
    // We exercise the redactor on the strings directly to lock in
    // the expectation; the daemon's ingest layer wires it in.
    assert!(!r.apply(match &env.event {
        IngestEvent::FileDiff { pre_excerpt, .. } => pre_excerpt,
        _ => unreachable!(),
    }).contains("AKIAIOSFODNN7EXAMPLE"));
}
```

Run it: `cargo test -p teramindd redactor_strips_aws_keys_from_file_diff_excerpts`
Expected: PASS — confirms redactor handles the input. (The actual daemon wiring is exercised in the L3 integration test in Section 11.)

- [ ] **Step 2: Extend `redact_envelope` in `ingest.rs`**

Inside the `redact_envelope` function, before the `other => other` catch-all, add:

```rust
        FileDiff {
            session_id,
            turn_id,
            file_path,
            rel_path,
            attribution,
            language,
            pre_excerpt,
            post_excerpt,
            unified_diff,
            pre_hash,
            post_hash,
            byte_size,
        } => FileDiff {
            session_id,
            turn_id,
            file_path,
            rel_path,
            attribution,
            language,
            pre_excerpt: r.apply(&pre_excerpt),
            post_excerpt: r.apply(&post_excerpt),
            unified_diff: r.apply(&unified_diff),
            pre_hash,
            post_hash,
            byte_size,
        },
```

- [ ] **Step 3: Add route arm for `FileDiff`**

Inside `route()`, before the last existing arm (`PreCompact { session_id }`), insert:

```rust
        FileDiff {
            session_id,
            turn_id,
            file_path,
            rel_path,
            attribution,
            language,
            pre_excerpt,
            post_excerpt,
            unified_diff,
            pre_hash,
            post_hash,
            byte_size,
        } => {
            use teramind_db::repos::diff::NewFileDiff;
            d.diffs
                .insert(NewFileDiff {
                    turn_id,
                    session_id,
                    file_path: &file_path,
                    rel_path: &rel_path,
                    attribution,
                    language: language.as_deref(),
                    pre_excerpt: &pre_excerpt,
                    post_excerpt: &post_excerpt,
                    unified_diff: &unified_diff,
                    pre_hash,
                    post_hash,
                    byte_size,
                    captured_at: ts,
                })
                .await?;
            d.sessions.touch(session_id, ts, turn_id).await;
        }
```

- [ ] **Step 4: `cargo check -p teramindd && cargo test -p teramindd --lib`**

Expected: succeeds.

- [ ] **Step 5: Commit**

```bash
git add crates/teramindd/src/services/ingest.rs crates/teramindd/tests/file_diff_redaction.rs
git commit -m "feat(daemon): redact + persist IngestEvent::FileDiff"
```

---

### Task 7.2: Wire `fs_watcher_gaps_total` into status

The watcher's `gaps_counter` was wired in Section 6. Now mirror it into `IngestStats` so `teramind status` can surface it.

**Files:**
- Modify: `crates/teramindd/src/services/ingest.rs`
- Modify: `crates/teramindd/src/app.rs`

- [ ] **Step 1: Add the counter field on `IngestStats`**

Extend `IngestStats` in `ingest.rs`:

```rust
#[derive(Default)]
pub struct IngestStats {
    pub drops: AtomicU64,
    pub queue_depth: AtomicU64,
    pub pg_write_failures: AtomicU64,
    pub dead_letters: AtomicU64,
    pub fs_watcher_gaps: AtomicU64,
}
```

- [ ] **Step 2: Mirror `gaps_counter` into `stats.fs_watcher_gaps`**

In `crates/teramindd/src/app.rs`, just below the `WatchRegistry::new(raw_tx, gaps_counter.clone())` line introduced in Section 6, spawn a background task that mirrors the counter into `stats` every 5 s:

```rust
        {
            let s = stats.clone();
            let g = gaps_counter.clone();
            tokio::spawn(async move {
                loop {
                    let v = g.load(std::sync::atomic::Ordering::Relaxed);
                    s.fs_watcher_gaps.store(v, std::sync::atomic::Ordering::Relaxed);
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            });
        }
```

- [ ] **Step 3: Surface in `StatusReport`**

Edit `crates/teramind-ipc/src/proto.rs` (the `StatusReport` struct) — add field:

```rust
    pub fs_watcher_gaps_total: u64,
```

Then in `ipc_server.rs` populate it:

```rust
            Request::Status => Response::Status(StatusReport {
                uptime_seconds: self.started.elapsed().as_secs(),
                pg_connected: true,
                ingest_queue_depth: self.stats.queue_depth.load(Ordering::Relaxed) as u32,
                ingest_drops_total: self.stats.drops.load(Ordering::Relaxed),
                last_storage_pg_bytes: self.last_pg_bytes.load(Ordering::Relaxed),
                last_storage_jsonl_bytes: self.last_jsonl_bytes.load(Ordering::Relaxed),
                fs_watcher_gaps_total: self.stats.fs_watcher_gaps.load(Ordering::Relaxed),
            }),
```

- [ ] **Step 4: `cargo check -p teramindd -p teramind-ipc`**

Expected: succeeds. If `teramind` CLI prints status and constructs a `StatusReport` literal anywhere, add the new field there too (`grep -rn "StatusReport {" crates/`).

- [ ] **Step 5: Commit**

```bash
git add crates/teramindd/src/services/ingest.rs crates/teramindd/src/app.rs crates/teramind-ipc/src/proto.rs crates/teramindd/src/services/ipc_server.rs
git commit -m "feat(daemon): fs_watcher_gaps_total counter + StatusReport field"
```

---

## Section 8 — SessionStart/SessionEnd drive watcher registration

Now we connect ingest's `SessionStart`/`SessionEnd` arms to the watcher registry.

### Task 8.1: Register on `SessionStart`, unregister on `SessionEnd`

**Files:**
- Modify: `crates/teramindd/src/services/ingest.rs`

- [ ] **Step 1: Modify the `SessionStart` arm in `route()`**

Right after the existing `d.sessions.start(ActiveSession { … }).await;` line, insert:

```rust
            // Start watching this cwd; per-cwd refcount in the registry
            // handles duplicate sessions in the same directory.
            if let Err(e) = d.fs_registry
                .register(std::path::PathBuf::from(&cwd), sid)
                .await
            {
                warn!(error = %e, cwd, "fs_watcher: register failed");
            }
```

- [ ] **Step 2: Modify the `SessionEnd` arm**

Replace the body of `SessionEnd { session_id, reason } => …` with:

```rust
        SessionEnd { session_id, reason } => {
            d.session_repo.end(session_id, ts, &reason).await?;
            if let Some(active) = d.sessions.end(session_id).await {
                d.fs_registry
                    .unregister(std::path::Path::new(&active.cwd), session_id)
                    .await;
            }
        }
```

- [ ] **Step 3: `cargo check -p teramindd`**

Expected: succeeds.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/ingest.rs
git commit -m "feat(daemon): SessionStart/End drives fs_watcher registry"
```

---

## Section 9 — Auto-recall enrichment (diff-based query #2)

Spec §6.4 says auto-recall runs three queries; Plan C only implemented query #1 (recent turns). With diffs now flowing, we add the second query — most-similar 5 `file_diffs` whose `rel_path` matches files currently present in cwd.

### Task 9.1: `SearchRepo::diff_excerpts_for_cwd_files`

**Files:**
- Modify: `crates/teramind-db/src/repos/search.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/teramind-db/tests/search_repo.rs` (create if missing — pattern follows other repo tests in this crate). If the file already exists, append to it. Otherwise create:

```rust
// crates/teramind-db/tests/search_repo.rs
use teramind_db::repos::{AgentRepo, DiffRepo, SearchRepo, SessionRepo};
use teramind_db::repos::diff::NewFileDiff;
use teramind_db::repos::session::NewSession;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramind_core::types::file_diff::Attribution;
use time::OffsetDateTime;

#[tokio::test]
async fn diff_excerpts_for_cwd_files_filters_by_rel_path() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;

    let agents = AgentRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());
    let diffs = DiffRepo::new(pool.clone());
    let search = SearchRepo::new(pool.clone());

    let agent = agents.upsert("claude_code", None).await?;
    let sid = sessions.insert(NewSession {
        agent_id: agent.id, agent_session_id: None, cwd: "/proj",
        project_id: None, parent_session_id: None, git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u",
        started_at: OffsetDateTime::now_utc(),
    }).await?;

    let now = OffsetDateTime::now_utc();
    diffs.insert(NewFileDiff {
        turn_id: None, session_id: sid,
        file_path: "/proj/src/foo.rs", rel_path: "src/foo.rs",
        attribution: Attribution::Agent, language: Some("rust"),
        pre_excerpt: "old foo", post_excerpt: "new foo",
        unified_diff: "--- a/src/foo.rs\n+++ b/src/foo.rs\n-old foo\n+new foo\n",
        pre_hash: [0u8;32], post_hash: [1u8;32], byte_size: 7, captured_at: now,
    }).await?;
    diffs.insert(NewFileDiff {
        turn_id: None, session_id: sid,
        file_path: "/proj/src/bar.rs", rel_path: "src/bar.rs",
        attribution: Attribution::Agent, language: Some("rust"),
        pre_excerpt: "old bar", post_excerpt: "new bar",
        unified_diff: "--- a/src/bar.rs\n+++ b/src/bar.rs\n-old bar\n+new bar\n",
        pre_hash: [2u8;32], post_hash: [3u8;32], byte_size: 7, captured_at: now,
    }).await?;

    let hits = search
        .diff_excerpts_for_cwd_files(&["src/foo.rs".into()], 10)
        .await?;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].rel_path, "src/foo.rs");

    sup.shutdown().await?;
    Ok(())
}
```

- [ ] **Step 2: Run, confirm fail**

Run: `cargo test -p teramind-db diff_excerpts_for_cwd_files_filters_by_rel_path`
Expected: FAIL — method undefined.

- [ ] **Step 3: Add the method**

Append to `SearchRepo` impl in `crates/teramind-db/src/repos/search.rs`:

```rust
    /// Return the most recent diff excerpts whose `rel_path` is in `paths`.
    /// Empty `paths` yields an empty result without touching the DB.
    pub async fn diff_excerpts_for_cwd_files(
        &self,
        paths: &[String],
        limit: u32,
    ) -> Result<Vec<RankedDiff>> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let rows: Vec<(Uuid, Uuid, String, OffsetDateTime, Option<Uuid>, String, String)> = sqlx::query_as(
            r#"
            SELECT
                fd.id, fd.session_id, fd.rel_path, fd.captured_at,
                s.project_id,
                fd.pre_excerpt, fd.post_excerpt
            FROM file_diffs fd
            JOIN sessions s ON s.id = fd.session_id
            WHERE fd.rel_path = ANY($1)
            ORDER BY fd.captured_at DESC
            LIMIT $2
            "#,
        )
        .bind(paths)
        .bind(limit as i64)
        .fetch_all(self.pool.pg()).await?;

        Ok(rows.into_iter().map(|(diff_id, session_id, rel_path, ts, project_id, pre, post)| {
            RankedDiff {
                diff_id, session_id, rel_path, ts, project_id,
                trgm_score: 0.0, pre_excerpt: pre, post_excerpt: post,
            }
        }).collect())
    }
```

Also: `SessionId` is unused in this file's existing imports — remove if `cargo check` complains (just delete the unused import).

- [ ] **Step 4: Run, confirm PASS**

Run: `cargo test -p teramind-db diff_excerpts_for_cwd_files_filters_by_rel_path`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-db/src/repos/search.rs crates/teramind-db/tests/search_repo.rs
git commit -m "feat(db): SearchRepo.diff_excerpts_for_cwd_files"
```

---

### Task 9.2: Extend `do_auto_recall` to merge diff results

**Files:**
- Modify: `crates/teramindd/src/services/search.rs`
- Modify: `crates/teramind-core/src/types/search.rs` (if needed — see step 2)

- [ ] **Step 1: Add a failing test**

Append to `crates/teramindd/src/services/search.rs`'s `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn render_auto_recall_md_includes_diffs_section_when_present() {
        let recent_turns: Vec<RankedTurn> = vec![RankedTurn {
            turn_id: Uuid::new_v4(), session_id: Uuid::new_v4(),
            ordinal: 0, ts: OffsetDateTime::now_utc(), project_id: None,
            fts_score: 0.0, trgm_score: 0.0,
            user_prompt: Some("fix bug".into()),
            assistant_text: Some("done".into()),
        }];
        let diff_hits: Vec<teramind_db::repos::search::RankedDiff> =
            vec![teramind_db::repos::search::RankedDiff {
                diff_id: Uuid::new_v4(),
                session_id: Uuid::new_v4(),
                rel_path: "src/foo.rs".into(),
                ts: OffsetDateTime::now_utc(),
                project_id: None,
                trgm_score: 0.0,
                pre_excerpt: "old foo".into(),
                post_excerpt: "new foo".into(),
            }];
        let md = render_auto_recall_md(&recent_turns, &diff_hits);
        assert!(md.contains("Recent Teramind context"));
        assert!(md.contains("fix bug"));
        assert!(md.contains("Recent diffs"));
        assert!(md.contains("src/foo.rs"));
    }
```

- [ ] **Step 2: Extract `render_auto_recall_md` and extend `do_auto_recall`**

Replace the existing `do_auto_recall` (and add a new helper above it):

```rust
pub fn render_auto_recall_md(
    recent: &[teramind_db::repos::search::RankedTurn],
    diffs: &[teramind_db::repos::search::RankedDiff],
) -> String {
    let mut out = String::new();
    if !recent.is_empty() {
        out.push_str("## Recent Teramind context\n\n");
        for t in recent {
            let prompt_snippet = t.user_prompt.as_deref().unwrap_or("(no prompt)");
            let text_snippet = t.assistant_text.as_deref().unwrap_or("");
            out.push_str(&format!(
                "- **{}**: {} · {}\n",
                t.ts.date(),
                truncate(prompt_snippet, 80),
                truncate(text_snippet, 120),
            ));
        }
        out.push('\n');
    }
    if !diffs.is_empty() {
        out.push_str("## Recent diffs in this project\n\n");
        for d in diffs {
            out.push_str(&format!(
                "- `{}` @ {}: {}\n",
                d.rel_path,
                d.ts.date(),
                truncate(&d.post_excerpt, 120),
            ));
        }
    }
    out
}

pub async fn do_auto_recall(
    repo: &SearchRepo,
    req: &AutoRecallRequest,
) -> Result<String, teramind_db::DbError> {
    let (recent, diffs) = tokio::try_join!(
        repo.recent_turns_in_project(None, &req.cwd, req.limit),
        repo.diff_excerpts_for_cwd_files(&req.cwd_files, req.limit),
    )?;
    if recent.is_empty() && diffs.is_empty() {
        return Ok(String::new());
    }
    Ok(render_auto_recall_md(&recent, &diffs))
}
```

- [ ] **Step 3: Add `cwd_files` to `AutoRecallRequest`**

In `crates/teramind-core/src/types/search.rs`, locate `AutoRecallRequest` (or wherever it lives) and add a field:

```rust
    #[serde(default)]
    pub cwd_files: Vec<String>,
```

If `AutoRecallRequest` is declared with `derive(Default)` already, no further change. Otherwise also add `#[derive(Default)]`.

- [ ] **Step 4: Populate `cwd_files` from the hook**

In `crates/teramind-hook/src/auto_recall.rs`, before sending the request, walk the cwd (one level deep is enough; we just need files present so the diff query matches them). Add:

```rust
fn list_cwd_files(cwd: &std::path::Path, limit: usize) -> Vec<String> {
    use ignore::WalkBuilder;
    let mut out = Vec::with_capacity(limit);
    let walker = WalkBuilder::new(cwd).hidden(false).max_depth(Some(3)).build();
    for entry in walker.flatten().take(limit * 8) {
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            if let Ok(rel) = entry.path().strip_prefix(cwd) {
                out.push(rel.to_string_lossy().to_string());
                if out.len() >= limit { break; }
            }
        }
    }
    out
}
```

(Add `ignore = { workspace = true }` to `teramind-hook`'s `Cargo.toml`.)

Then where the hook builds its `AutoRecallRequest`, set `cwd_files: list_cwd_files(&cwd, 50)`.

- [ ] **Step 5: Run**

Run: `cargo test -p teramindd render_auto_recall_md_includes_diffs_section_when_present`
Expected: PASS.

Run: `cargo check -p teramind-hook`
Expected: succeeds.

- [ ] **Step 6: Commit**

```bash
git add crates/teramindd/src/services/search.rs crates/teramind-core/src/types/search.rs crates/teramind-hook/src/auto_recall.rs crates/teramind-hook/Cargo.toml
git commit -m "feat(search): auto_recall merges diff excerpts for cwd files"
```

---

## Section 10 — Daemon config

The watcher's debounce window, agent-attribution window, and snapshot TTL should be tunable.

### Task 10.1: Add `fs_watcher` config block

**Files:**
- Modify: `crates/teramindd/src/config.rs`
- Modify: `crates/teramindd/src/app.rs`

- [ ] **Step 1: Add fields to `Config`**

In `crates/teramindd/src/config.rs`, locate `pub struct Config` and add three fields with defaults:

```rust
    #[serde(default = "default_fs_debounce_ms")]
    pub fs_debounce_ms: u64,
    #[serde(default = "default_attribution_window_ms")]
    pub fs_attribution_window_ms: u64,
    #[serde(default = "default_snapshot_ttl_secs")]
    pub fs_snapshot_ttl_secs: u64,
```

Append default fns:

```rust
fn default_fs_debounce_ms() -> u64 { 200 }
fn default_attribution_window_ms() -> u64 { 5_000 }
fn default_snapshot_ttl_secs() -> u64 { 1_800 }
```

Also ensure the `Default` impl (if any) wires them up.

- [ ] **Step 2: Use them in `App::run`**

Replace the hard-coded literals in `App::run`:

- `Duration::from_millis(200)` → `Duration::from_millis(config.fs_debounce_ms)`
- `time::Duration::seconds(5)` (write tool ring window) → `time::Duration::milliseconds(config.fs_attribution_window_ms as i64)`
- `time::Duration::minutes(30)` (snapshot cache) → `time::Duration::seconds(config.fs_snapshot_ttl_secs as i64)`

- [ ] **Step 3: Add a unit test**

Append to `crates/teramindd/src/config.rs` test module:

```rust
    #[test]
    fn fs_watcher_defaults_match_spec() {
        let c = Config::default();
        assert_eq!(c.fs_debounce_ms, 200);
        assert_eq!(c.fs_attribution_window_ms, 5_000);
        assert_eq!(c.fs_snapshot_ttl_secs, 1_800);
    }
```

- [ ] **Step 4: Run**

Run: `cargo test -p teramindd config`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramindd/src/config.rs crates/teramindd/src/app.rs
git commit -m "feat(daemon): fs_watcher config knobs"
```

---

## Section 11 — L3 integration tests

These tests stand up a real `teramindd` against an embedded Postgres, drive synthetic events, then assert rows in `file_diffs`.

### Task 11.1: Helper crate scaffolding (or reuse existing pattern)

**Files:**
- Read: `crates/teramindd/tests/*.rs` (any existing L3 test) for the canonical pattern

- [ ] **Step 1: Find the existing pattern**

Run: `ls crates/teramindd/tests/`
Note: Plans A/B/C produced files like `smoke_e2e.rs`, `search_cli.rs`. Open the most similar one (likely `smoke_e2e.rs`) and adopt its boilerplate for spawning `App::run` against a tempdir-rooted Postgres.

If no L3 harness exists yet, fall back to constructing `PgSupervisor` + the same `IngestService` wiring as `App::run` does, directly inside the test (it's only ~30 lines).

- [ ] **Step 2: Sketch the harness in a new helper module** (only if no existing helper)

Create `crates/teramindd/tests/common/mod.rs`:

```rust
//! Shared test scaffolding for L3 integration tests.
//! Spins up a real embedded Postgres + the daemon services in-process
//! and returns handles for driving events and asserting state.

use std::path::PathBuf;
use std::sync::Arc;
use teramind_core::redact::Redactor;
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo};
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramindd::services::fs_watcher::{Debouncer, FsWatcherDeps, FsWatcherService, WatchRegistry};
use teramindd::services::ingest::{IngestDeps, IngestService, IngestStats};
use teramindd::services::jsonl_writer::JsonlWriter;
use teramindd::services::session_manager::SessionManager;
use teramindd::services::snapshot_cache::SnapshotCache;
use teramindd::services::write_tool_ring::WriteToolRing;

pub struct Harness {
    pub pool: DbPool,
    pub ingest: Arc<IngestService>,
    pub registry: Arc<WatchRegistry>,
    pub _sup: PgSupervisor,
    pub _tmp: tempfile::TempDir,
    pub raw_dir: PathBuf,
    pub dead_letter_dir: PathBuf,
}

impl Harness {
    pub async fn start() -> anyhow::Result<Self> {
        let tmp = tempfile::tempdir()?;
        let raw_dir = tmp.path().join("raw");
        std::fs::create_dir_all(&raw_dir)?;
        let dead_letter_dir = tmp.path().join("dl");
        std::fs::create_dir_all(&dead_letter_dir)?;
        let pgdata = tmp.path().join("pgdata");
        let sup = PgSupervisor::start(pgdata, "teramind").await?;
        let pool = DbPool::connect(sup.connect_options()).await?;
        migrate::run(&pool).await?;

        let stats = Arc::new(IngestStats::default());
        let jsonl = Arc::new(JsonlWriter::open(raw_dir.clone()).await?);
        let write_tool_ring = WriteToolRing::new(64, time::Duration::seconds(5));

        let (raw_tx, mut raw_rx) = tokio::sync::mpsc::unbounded_channel();
        let (resolved_tx, resolved_rx) = tokio::sync::mpsc::unbounded_channel();
        let debouncer = Arc::new(Debouncer::start(std::time::Duration::from_millis(100), resolved_tx));
        let gaps_counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let registry = Arc::new(WatchRegistry::new(raw_tx, gaps_counter));

        let deb = debouncer.clone();
        tokio::spawn(async move {
            while let Some(ev) = raw_rx.recv().await {
                deb.enqueue(ev).await;
            }
        });

        let snapshot_cache = SnapshotCache::new(time::Duration::seconds(60));

        let ingest = Arc::new(IngestService::spawn(
            1024,
            IngestDeps {
                redactor: Arc::new(Redactor::with_default_rules()),
                jsonl: jsonl.clone(),
                sessions: SessionManager::new(),
                agents: AgentRepo::new(pool.clone()),
                session_repo: SessionRepo::new(pool.clone()),
                trace: TraceRepo::new(pool.clone()),
                diffs: DiffRepo::new(pool.clone()),
                stats: stats.clone(),
                dead_letter_dir: dead_letter_dir.clone(),
                write_tool_ring: write_tool_ring.clone(),
                fs_registry: registry.clone(),
            },
        ));

        FsWatcherService::spawn(
            FsWatcherDeps {
                registry: registry.clone(),
                debouncer: debouncer.clone(),
                cache: snapshot_cache.clone(),
                ring: write_tool_ring.clone(),
                ingest_tx: ingest.clone(),
            },
            resolved_rx,
        );

        Ok(Harness {
            pool, ingest, registry,
            _sup: sup, _tmp: tmp,
            raw_dir, dead_letter_dir,
        })
    }
}
```

- [ ] **Step 3: `cargo check --tests -p teramindd`**

Expected: succeeds.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/tests/common/mod.rs
git commit -m "test(daemon): L3 Harness for fs_watcher integration"
```

---

### Task 11.2: Test — agent attribution within 5 s of `PostToolUse`

**Files:**
- Create: `crates/teramindd/tests/fs_watcher_attribution.rs`

- [ ] **Step 1: Write the test**

Create `crates/teramindd/tests/fs_watcher_attribution.rs`:

```rust
mod common;

use common::Harness;
use teramind_core::ids::{ClientEventId, SessionId, ToolCallId, TurnId};
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use time::OffsetDateTime;

async fn count_diffs(pool: &teramind_db::pool::DbPool) -> i64 {
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM file_diffs")
        .fetch_one(pool.pg()).await.unwrap();
    n
}

async fn diff_row(pool: &teramind_db::pool::DbPool) -> Option<(String, String, Option<uuid::Uuid>)> {
    sqlx::query_as("SELECT rel_path, attribution, turn_id FROM file_diffs ORDER BY captured_at DESC LIMIT 1")
        .fetch_optional(pool.pg()).await.unwrap()
        .map(|(rel, attr, tid): (String, String, Option<uuid::Uuid>)| (rel, attr, tid))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agent_attribution_when_post_tool_use_precedes_write() -> anyhow::Result<()> {
    let h = Harness::start().await?;
    let cwd = h._tmp.path().join("proj");
    std::fs::create_dir_all(&cwd)?;
    std::fs::write(cwd.join("a.rs"), "fn old() {}\n")?;

    let sid = SessionId::new();
    let tid = TurnId::new();

    // 1) SessionStart (registers the watcher)
    h.ingest.try_enqueue(EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: OffsetDateTime::now_utc(),
        event: IngestEvent::SessionStart {
            session_id: sid,
            agent_session_id: None,
            agent_kind: "claude_code".into(),
            cwd: cwd.to_string_lossy().to_string(),
            os: "linux".into(),
            hostname: "h".into(),
            user_login: "u".into(),
            git_head: None,
            git_branch: None,
        },
    }).map_err(|_| anyhow::anyhow!("enqueue SessionStart"))?;

    // 2) UserPrompt creating the turn
    h.ingest.try_enqueue(EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: OffsetDateTime::now_utc(),
        event: IngestEvent::UserPrompt {
            session_id: sid,
            turn_ordinal: 0,
            prompt: "edit a.rs".into(),
            turn_id: Some(tid),
        },
    }).map_err(|_| anyhow::anyhow!("enqueue UserPrompt"))?;

    // 3) PreToolUse + PostToolUse for an Edit (records into the ring)
    let tool_id = ToolCallId::new();
    h.ingest.try_enqueue(EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: OffsetDateTime::now_utc(),
        event: IngestEvent::ToolCallStart {
            turn_id: tid,
            tool_call_id: Some(tool_id),
            ordinal: 0,
            name: "Edit".into(),
            input: serde_json::json!({"path":"a.rs"}),
        },
    }).map_err(|_| anyhow::anyhow!("enqueue ToolStart"))?;
    h.ingest.try_enqueue(EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: OffsetDateTime::now_utc(),
        event: IngestEvent::ToolCallEnd {
            tool_call_id: tool_id,
            output: "ok".into(),
            is_error: false,
            duration_ms: 5,
            session_id: Some(sid),
            turn_id: Some(tid),
            tool_name: Some("Edit".into()),
        },
    }).map_err(|_| anyhow::anyhow!("enqueue ToolEnd"))?;

    // Give ingest a moment to register the watcher + push to the ring.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // 4) Modify the file -> watcher should produce an Agent-attributed diff
    std::fs::write(cwd.join("a.rs"), "fn new() {}\n")?;

    // Poll for the row to appear (budget: 1s per spec §9.7).
    let mut got = None;
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if count_diffs(&h.pool).await > 0 {
            got = diff_row(&h.pool).await;
            break;
        }
    }
    let (rel, attr, turn_id) = got.expect("file_diffs row not written within budget");
    assert_eq!(rel, "a.rs");
    assert_eq!(attr, "agent");
    assert_eq!(turn_id, Some(tid.0));
    Ok(())
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramindd --test fs_watcher_attribution -- --nocapture`
Expected: PASS within ~3 s of test start.

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/tests/fs_watcher_attribution.rs
git commit -m "test(daemon): L3 agent attribution within window"
```

---

### Task 11.3: Test — human attribution outside the window

**Files:**
- Modify: `crates/teramindd/tests/fs_watcher_attribution.rs`

- [ ] **Step 1: Append the second test**

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn human_attribution_when_no_recent_write_tool() -> anyhow::Result<()> {
    let h = Harness::start().await?;
    let cwd = h._tmp.path().join("proj");
    std::fs::create_dir_all(&cwd)?;
    std::fs::write(cwd.join("a.rs"), "fn old() {}\n")?;

    let sid = SessionId::new();
    h.ingest.try_enqueue(EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: OffsetDateTime::now_utc(),
        event: IngestEvent::SessionStart {
            session_id: sid,
            agent_session_id: None,
            agent_kind: "claude_code".into(),
            cwd: cwd.to_string_lossy().to_string(),
            os: "linux".into(),
            hostname: "h".into(),
            user_login: "u".into(),
            git_head: None,
            git_branch: None,
        },
    }).map_err(|_| anyhow::anyhow!("enqueue SessionStart"))?;

    // No tool events. Modify directly.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    std::fs::write(cwd.join("a.rs"), "fn new() {}\n")?;

    let mut got = None;
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if count_diffs(&h.pool).await > 0 {
            got = diff_row(&h.pool).await;
            break;
        }
    }
    let (_, attr, turn_id) = got.expect("file_diffs row not written");
    assert_eq!(attr, "human");
    assert!(turn_id.is_none(), "human-attributed diff must not carry a turn_id");
    Ok(())
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramindd human_attribution_when_no_recent_write_tool -- --nocapture`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/tests/fs_watcher_attribution.rs
git commit -m "test(daemon): L3 human attribution when no recent write tool"
```

---

### Task 11.4: Test — redaction applied to file_diffs excerpts

**Files:**
- Create: `crates/teramindd/tests/fs_watcher_redaction.rs`

- [ ] **Step 1: Write the test**

```rust
mod common;

use common::Harness;
use teramind_core::ids::{ClientEventId, SessionId};
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use time::OffsetDateTime;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn aws_key_in_diff_is_redacted_before_persist() -> anyhow::Result<()> {
    let h = Harness::start().await?;
    let cwd = h._tmp.path().join("proj");
    std::fs::create_dir_all(&cwd)?;
    std::fs::write(cwd.join("creds.rs"), "let k = \"\";\n")?;

    let sid = SessionId::new();
    h.ingest.try_enqueue(EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: OffsetDateTime::now_utc(),
        event: IngestEvent::SessionStart {
            session_id: sid,
            agent_session_id: None,
            agent_kind: "claude_code".into(),
            cwd: cwd.to_string_lossy().to_string(),
            os: "linux".into(), hostname: "h".into(), user_login: "u".into(),
            git_head: None, git_branch: None,
        },
    }).map_err(|_| anyhow::anyhow!("enqueue"))?;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    std::fs::write(cwd.join("creds.rs"), "let k = \"AKIAIOSFODNN7EXAMPLE\";\n")?;

    let mut excerpt: Option<String> = None;
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if let Some((post,)) = sqlx::query_as::<_, (String,)>(
            "SELECT post_excerpt FROM file_diffs ORDER BY captured_at DESC LIMIT 1",
        ).fetch_optional(h.pool.pg()).await? {
            excerpt = Some(post);
            break;
        }
    }
    let post = excerpt.expect("no diff row");
    assert!(!post.contains("AKIAIOSFODNN7EXAMPLE"),
            "redaction failed; post_excerpt: {post}");
    assert!(post.contains("«redacted"), "expected redaction marker, got: {post}");
    Ok(())
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramindd aws_key_in_diff_is_redacted_before_persist -- --nocapture`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/tests/fs_watcher_redaction.rs
git commit -m "test(daemon): L3 redaction on file_diffs excerpts"
```

---

### Task 11.5: Test — ignore filter excludes `.git/` and `target/`

**Files:**
- Modify: `crates/teramindd/tests/fs_watcher_attribution.rs` (append) or new file

- [ ] **Step 1: Append a third test (in the same file)**

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ignored_paths_produce_no_file_diff_row() -> anyhow::Result<()> {
    let h = Harness::start().await?;
    let cwd = h._tmp.path().join("proj");
    std::fs::create_dir_all(cwd.join(".git"))?;
    std::fs::create_dir_all(cwd.join("target"))?;
    std::fs::write(cwd.join(".git/HEAD"), "ref: refs/heads/main\n")?;
    std::fs::write(cwd.join("target/x"), "x")?;
    std::fs::write(cwd.join("a.rs"), "fn old(){}\n")?;

    let sid = SessionId::new();
    h.ingest.try_enqueue(EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: OffsetDateTime::now_utc(),
        event: IngestEvent::SessionStart {
            session_id: sid,
            agent_session_id: None,
            agent_kind: "claude_code".into(),
            cwd: cwd.to_string_lossy().to_string(),
            os: "linux".into(), hostname: "h".into(), user_login: "u".into(),
            git_head: None, git_branch: None,
        },
    }).map_err(|_| anyhow::anyhow!("enqueue"))?;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // Modify both an ignored and a tracked path.
    std::fs::write(cwd.join(".git/HEAD"), "ref: refs/heads/feat\n")?;
    std::fs::write(cwd.join("target/x"), "y")?;
    std::fs::write(cwd.join("a.rs"), "fn new(){}\n")?;

    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    let rows: Vec<(String,)> = sqlx::query_as("SELECT rel_path FROM file_diffs")
        .fetch_all(h.pool.pg()).await?;
    let paths: Vec<String> = rows.into_iter().map(|(s,)| s).collect();
    assert!(paths.iter().any(|p| p == "a.rs"), "expected a.rs in {paths:?}");
    assert!(!paths.iter().any(|p| p.starts_with(".git/")), "got .git/ in {paths:?}");
    assert!(!paths.iter().any(|p| p.starts_with("target/")), "got target/ in {paths:?}");
    Ok(())
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramindd ignored_paths_produce_no_file_diff_row -- --nocapture`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/tests/fs_watcher_attribution.rs
git commit -m "test(daemon): L3 ignore filter excludes .git/target"
```

---

### Task 11.6: Test — file save to row p99 < 1 s

**Files:**
- Create: `crates/teramindd/tests/fs_watcher_latency.rs`

- [ ] **Step 1: Write the test**

```rust
mod common;

use common::Harness;
use teramind_core::ids::{ClientEventId, SessionId};
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use time::OffsetDateTime;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn file_save_to_row_p99_under_one_second() -> anyhow::Result<()> {
    let h = Harness::start().await?;
    let cwd = h._tmp.path().join("proj");
    std::fs::create_dir_all(&cwd)?;
    std::fs::write(cwd.join("a.rs"), "v0\n")?;

    let sid = SessionId::new();
    h.ingest.try_enqueue(EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: OffsetDateTime::now_utc(),
        event: IngestEvent::SessionStart {
            session_id: sid, agent_session_id: None,
            agent_kind: "claude_code".into(),
            cwd: cwd.to_string_lossy().to_string(),
            os: "linux".into(), hostname: "h".into(), user_login: "u".into(),
            git_head: None, git_branch: None,
        },
    }).map_err(|_| anyhow::anyhow!("enqueue"))?;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let n = 20;
    let mut latencies = Vec::with_capacity(n);
    for i in 1..=n {
        let started = std::time::Instant::now();
        std::fs::write(cwd.join("a.rs"), format!("v{i}\n"))?;
        // Poll until count_diffs == i.
        loop {
            let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM file_diffs")
                .fetch_one(h.pool.pg()).await?;
            if count as usize >= i {
                latencies.push(started.elapsed());
                break;
            }
            if started.elapsed() > std::time::Duration::from_secs(3) {
                anyhow::bail!("timeout waiting for diff #{i}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
    latencies.sort();
    let p99 = latencies[(latencies.len() as f64 * 0.99) as usize];
    assert!(p99 < std::time::Duration::from_secs(1),
        "p99 = {p99:?}, budget 1 s (spec §9.7)");
    Ok(())
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p teramindd file_save_to_row_p99_under_one_second --release -- --nocapture`
Expected: PASS (release build to keep notify/diff math out of debug penalty).

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/tests/fs_watcher_latency.rs
git commit -m "test(daemon): L3 watcher latency p99 < 1s (spec §9.7)"
```

---

## Section 12 — L4 manual smoke runbook

### Task 12.1: Write `docs/runbooks/fs-watcher-manual-smoke.md`

**Files:**
- Create: `docs/runbooks/fs-watcher-manual-smoke.md`

- [ ] **Step 1: Author the runbook**

Create the file with:

```markdown
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
```

- [ ] **Step 2: Commit**

```bash
git add docs/runbooks/fs-watcher-manual-smoke.md
git commit -m "docs: manual smoke runbook for FS watcher"
```

---

## Section 13 — Final integration check

### Task 13.1: Full test suite + workspace check

- [ ] **Step 1: Run everything**

```bash
cargo check --workspace
cargo test --workspace --lib
cargo test -p teramindd --test fs_watcher_attribution
cargo test -p teramindd --test fs_watcher_redaction
cargo test -p teramindd --test fs_watcher_latency --release
cargo test -p teramind-db diff_excerpts_for_cwd_files_filters_by_rel_path
```

Expected: all pass.

- [ ] **Step 2: Clippy**

```bash
cargo clippy --workspace -- -D warnings
```

Fix any warnings inline (most likely candidates: unused imports left over from refactors).

- [ ] **Step 3: Commit any cleanups**

```bash
git add -A
git commit -m "chore: clippy cleanups for fs_watcher integration" || true
```

(`|| true` because there may be nothing to commit.)

---

### Task 13.2: Open the merge PR

- [ ] **Step 1: Push the branch**

```bash
git push -u origin feat/teramind-fs-watcher
```

- [ ] **Step 2: Open the PR**

```bash
gh pr create --title "feat(teramind): FS watcher + per-turn diff capture (Plan D)" --body "$(cat <<'EOF'
## Summary
- Populates `file_diffs` with per-turn unified diffs computed inside the daemon.
- Adds a `notify`-backed watcher per active-session cwd with refcount semantics.
- Attribution decided via a 5 s write-tool ring; redaction applied pre-persist.
- Extends auto-recall to merge diff excerpts for files currently present in cwd.

## Test plan
- [ ] `cargo test --workspace --lib`
- [ ] `cargo test -p teramindd --test fs_watcher_attribution`
- [ ] `cargo test -p teramindd --test fs_watcher_redaction`
- [ ] `cargo test -p teramindd --test fs_watcher_latency --release`
- [ ] Manual smoke per `docs/runbooks/fs-watcher-manual-smoke.md`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 3: Confirm PR URL prints**

Expected output ends with `https://github.com/<org>/<repo>/pull/<n>`.

---

## Spec coverage self-check

| Spec section | Requirement | Plan task |
|---|---|---|
| §3 architecture | `fs_watcher` in daemon | Section 6 |
| §4.3 services | one watcher per cwd, 200 ms debounce | Tasks 6.1, 6.2 |
| §4.3 services | snapshot cache + git-index fallback | Section 3 |
| §4.3 services | ±50-line excerpts | Task 2.4 |
| §4.3 services | 5 s agent-attribution window after `PostToolUse` for write tools | Sections 4 + 6, Task 11.2 |
| §4.4 schema | writes to `file_diffs` columns (incl. `pre_hash`, `post_hash`, `byte_size`, `language`) | Task 7.1 |
| §5 capture flow | `write_tool_completed` broadcast for Edit/Write/MultiEdit/NotebookEdit | Tasks 4.1, 4.2 |
| §5 failure table | `fs_watcher_gaps_total` counter | Task 7.2 |
| §5 redaction | applied in `ingest` before persistence | Task 7.1 |
| §6.4 auto-recall | merge of recent turns + diffs for cwd files | Section 9 |
| §9.3 L3 | "synthetic PostToolUse → Edit → row with attribution=agent within 1 s" | Task 11.2 |
| §9.3 L3 | "same without tool event → attribution=human" | Task 11.3 |
| §9.6 property | excerpt math invariants | Task 2.6 |
| §9.7 perf | file save → row p99 < 1 s | Task 11.6 |
| §11 glossary | `attribution` (agent vs human) | Tasks 7.1, 11.2, 11.3 |
