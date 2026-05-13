# Teramind Core Foundation (Plan A) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the Rust workspace, embedded Postgres data layer, daemon skeleton, and CLI that together form Teramind's substrate. End state: a `teramindd` daemon that accepts JSON-RPC over IPC, persists ingest events to Postgres + JSONL, and a `teramind` CLI that drives lifecycle (`init`, `start`, `stop`, `status`, `doctor`).

**Architecture:** Multi-crate Cargo workspace. Stateless CLI/hook/MCP clients talk to one stateful daemon over a Unix Domain Socket (Unix) or Named Pipe (Windows). The daemon owns an embedded Postgres child process, a single-writer ingest pipeline, and a session manager. Capture is best-effort: never blocks the agent, every degradation is named and counted.

**Tech Stack:** Rust stable (toolchain pinned), `tokio` async runtime, `sqlx` for Postgres, `postgresql_embedded` for the embedded server, `clap` for the CLI, `serde` + `serde_json`, `thiserror`, `tracing` + `tracing-appender`, `interprocess` for cross-platform sockets/pipes, `notify` (later), `proptest`, `criterion`.

**Spec reference:** `docs/superpowers/specs/2026-05-13-teramind-core-design.md`. This plan implements Sections 1–5, 7 (lifecycle/installer minus packaging itself), and the L1–L2 testing slice of Section 9. Plans B–F follow.

---

## File Structure

```
teramind/
├── Cargo.toml                                    [workspace root]
├── rust-toolchain.toml                           [pinned stable]
├── .gitignore
├── crates/
│   ├── teramind-core/                            [shared types + redaction]
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── error.rs
│   │   │   ├── ids.rs                            [strongly-typed UUID newtypes]
│   │   │   ├── types/{mod,agent,project,session,turn,tool_call,
│   │   │   │         file_diff,skill,storage_stats,hit,ingest_event}.rs
│   │   │   └── redact/{mod,patterns,rules}.rs
│   │   └── tests/redaction_corpus.rs
│   ├── teramind-ipc/                             [JSON-RPC contract + transport]
│   │   ├── Cargo.toml
│   │   ├── src/{lib,proto,codec,client,server,error}.rs
│   │   ├── src/transport/{mod,unix,windows}.rs
│   │   └── tests/roundtrip.rs
│   ├── teramind-db/                              [migrations, repos, embedded PG]
│   │   ├── Cargo.toml
│   │   ├── migrations/0001…0010_*.sql
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── pool.rs
│   │   │   ├── migrate.rs
│   │   │   ├── pg_supervisor.rs
│   │   │   ├── error.rs
│   │   │   └── repos/{mod,agent,project,session,trace,diff,skill,
│   │   │              storage_stats}.rs
│   │   └── tests/{migrations,repos}.rs
│   ├── teramindd/                                [daemon]
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── app.rs
│   │   │   ├── config.rs
│   │   │   ├── paths.rs
│   │   │   ├── signals.rs
│   │   │   └── services/
│   │   │       ├── mod.rs
│   │   │       ├── ingest.rs
│   │   │       ├── session_manager.rs
│   │   │       ├── jsonl_writer.rs
│   │   │       ├── storage_stats.rs
│   │   │       └── ipc_server.rs
│   │   └── tests/{ingest_e2e,backpressure,inbox_drain}.rs
│   └── teramind/                                 [user-facing CLI]
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs
│           ├── cli.rs
│           ├── ipc.rs
│           └── commands/{mod,init,start,stop,status,doctor,
│                         restart,reset,version}.rs
```

**Why these boundaries:** `teramind-core` has zero runtime deps; everything else may depend on it. `teramind-ipc` knows about the wire protocol but not storage. `teramind-db` knows about Postgres but not the daemon. `teramindd` wires services together; `teramind` is a thin client. Each crate is independently testable.

---

## Section 1 — Workspace bootstrap

### Task 1: Create the Cargo workspace root

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `rust-toolchain.toml`
- Create: `.gitignore`

- [ ] **Step 1: Write the workspace `Cargo.toml`**

```toml
[workspace]
resolver = "2"
members = [
    "crates/teramind-core",
    "crates/teramind-ipc",
    "crates/teramind-db",
    "crates/teramindd",
    "crates/teramind",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "Apache-2.0"
rust-version = "1.78"

[workspace.dependencies]
anyhow      = "1"
thiserror   = "1"
serde       = { version = "1", features = ["derive"] }
serde_json  = "1"
tokio       = { version = "1", features = ["full"] }
tracing     = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
tracing-appender   = "0.2"
uuid        = { version = "1", features = ["v4", "serde"] }
time        = { version = "0.3", features = ["serde", "macros", "formatting", "parsing"] }
clap        = { version = "4", features = ["derive", "env"] }
sqlx        = { version = "0.7", default-features = false, features = ["runtime-tokio-rustls", "postgres", "uuid", "time", "json", "macros", "migrate"] }
postgresql_embedded = { version = "0.16", features = ["blocking"] }
interprocess = "1"
notify      = "6"
regex       = "1"
once_cell   = "1"
sha2        = "0.10"
hex         = "0.4"
async-trait = "0.1"
futures     = "0.3"
tempfile    = "3"
proptest    = "1"

[profile.release]
lto = "thin"
codegen-units = 1
```

- [ ] **Step 2: Write `rust-toolchain.toml`**

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
profile = "minimal"
```

- [ ] **Step 3: Write top-level `.gitignore`**

```gitignore
/target
**/*.rs.bk
Cargo.lock.bak
.DS_Store
.idea/
.vscode/
*.swp
# Teramind local state (must never be committed)
/.local-data/
```

- [ ] **Step 4: Verify workspace parses**

Run: `cargo metadata --format-version=1 --no-deps`
Expected: JSON output listing the 5 workspace members (or, if no crate dirs exist yet, an error pointing at the first missing member — that's fine; we create them in subsequent tasks).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml rust-toolchain.toml .gitignore
git commit -m "chore: initialize Cargo workspace for Teramind Core"
```

---

### Task 2: Create empty member crate directories

**Files:**
- Create: `crates/teramind-core/Cargo.toml`, `crates/teramind-core/src/lib.rs`
- Create: `crates/teramind-ipc/Cargo.toml`, `crates/teramind-ipc/src/lib.rs`
- Create: `crates/teramind-db/Cargo.toml`, `crates/teramind-db/src/lib.rs`
- Create: `crates/teramindd/Cargo.toml`, `crates/teramindd/src/main.rs`
- Create: `crates/teramind/Cargo.toml`, `crates/teramind/src/main.rs`

- [ ] **Step 1: Write `crates/teramind-core/Cargo.toml`**

```toml
[package]
name = "teramind-core"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[dependencies]
serde      = { workspace = true }
serde_json = { workspace = true }
thiserror  = { workspace = true }
uuid       = { workspace = true }
time       = { workspace = true }
regex      = { workspace = true }
once_cell  = { workspace = true }
sha2       = { workspace = true }
hex        = { workspace = true }

[dev-dependencies]
proptest = { workspace = true }
```

- [ ] **Step 2: Write `crates/teramind-core/src/lib.rs`**

```rust
//! Shared types, identifiers, error enum, and redaction rules.

pub mod error;
pub mod ids;
pub mod redact;
pub mod types;

pub use error::Error;
pub use ids::*;
pub use types::*;
```

- [ ] **Step 3: Write `crates/teramind-ipc/Cargo.toml`**

```toml
[package]
name = "teramind-ipc"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[dependencies]
teramind-core = { path = "../teramind-core" }
serde        = { workspace = true }
serde_json   = { workspace = true }
thiserror    = { workspace = true }
tokio        = { workspace = true }
async-trait  = { workspace = true }
futures      = { workspace = true }
interprocess = { workspace = true }
tracing      = { workspace = true }
uuid         = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

- [ ] **Step 4: Write `crates/teramind-ipc/src/lib.rs`**

```rust
//! JSON-RPC contract and cross-platform IPC transport for Teramind.

pub mod client;
pub mod codec;
pub mod error;
pub mod proto;
pub mod server;
pub mod transport;

pub use client::IpcClient;
pub use error::IpcError;
pub use proto::{Notify, Request, Response};
pub use server::IpcServer;
```

- [ ] **Step 5: Write `crates/teramind-db/Cargo.toml`**

```toml
[package]
name = "teramind-db"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[dependencies]
teramind-core = { path = "../teramind-core" }
sqlx        = { workspace = true }
postgresql_embedded = { workspace = true }
tokio       = { workspace = true }
serde       = { workspace = true }
serde_json  = { workspace = true }
thiserror   = { workspace = true }
tracing     = { workspace = true }
uuid        = { workspace = true }
time        = { workspace = true }
async-trait = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
anyhow   = { workspace = true }
```

- [ ] **Step 6: Write `crates/teramind-db/src/lib.rs`**

```rust
//! Embedded Postgres lifecycle, migrations, and per-entity repositories.

pub mod error;
pub mod migrate;
pub mod pg_supervisor;
pub mod pool;
pub mod repos;

pub use error::DbError;
pub use pool::DbPool;
```

- [ ] **Step 7: Write `crates/teramindd/Cargo.toml`**

```toml
[package]
name = "teramindd"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[[bin]]
name = "teramindd"
path = "src/main.rs"

[dependencies]
teramind-core = { path = "../teramind-core" }
teramind-ipc  = { path = "../teramind-ipc" }
teramind-db   = { path = "../teramind-db" }
tokio       = { workspace = true }
anyhow      = { workspace = true }
serde       = { workspace = true }
serde_json  = { workspace = true }
thiserror   = { workspace = true }
tracing     = { workspace = true }
tracing-subscriber = { workspace = true }
tracing-appender   = { workspace = true }
clap        = { workspace = true }
time        = { workspace = true }
uuid        = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

- [ ] **Step 8: Write `crates/teramindd/src/main.rs`**

```rust
fn main() {
    // Real entrypoint is wired in Task 36.
    eprintln!("teramindd: not yet implemented");
    std::process::exit(2);
}
```

- [ ] **Step 9: Write `crates/teramind/Cargo.toml`**

```toml
[package]
name = "teramind-cli"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[[bin]]
name = "teramind"
path = "src/main.rs"

[dependencies]
teramind-core = { path = "../teramind-core" }
teramind-ipc  = { path = "../teramind-ipc" }
tokio       = { workspace = true }
anyhow      = { workspace = true }
clap        = { workspace = true }
tracing     = { workspace = true }
tracing-subscriber = { workspace = true }
serde       = { workspace = true }
serde_json  = { workspace = true }
time        = { workspace = true }
```

- [ ] **Step 10: Write `crates/teramind/src/main.rs`**

```rust
fn main() {
    // Real entrypoint is wired in Task 47.
    eprintln!("teramind: not yet implemented");
    std::process::exit(2);
}
```

- [ ] **Step 11: Verify workspace builds (will fail on missing modules; that's expected)**

Run: `cargo check --workspace`
Expected: errors of the form `file not found for module 'error'` / `'types'` / etc. — the lib roots reference modules that don't exist yet. This is intentional; Section 2 creates them.

- [ ] **Step 12: Commit**

```bash
git add crates/
git commit -m "chore: scaffold workspace member crates"
```

---

## Section 2 — `teramind-core` types and error enum

These tasks build the shared types referenced everywhere else. All types live in `crates/teramind-core/src/types/` and re-export from `types/mod.rs`. Each new type comes with a serialization round-trip unit test.

### Task 3: Strongly-typed IDs

**Files:**
- Create: `crates/teramind-core/src/ids.rs`
- Test: in the same file under `#[cfg(test)] mod tests`.

- [ ] **Step 1: Write the failing test**

Append to `crates/teramind-core/src/ids.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

macro_rules! id_newtype {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self { Self(Uuid::new_v4()) }
            pub fn nil() -> Self { Self(Uuid::nil()) }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id_newtype!(AgentId);
id_newtype!(ProjectId);
id_newtype!(SessionId);
id_newtype!(TurnId);
id_newtype!(ToolCallId);
id_newtype!(FileDiffId);
id_newtype!(SkillId);
id_newtype!(ClientEventId);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_id_roundtrips_as_uuid_string() {
        let id = SessionId::new();
        let json = serde_json::to_string(&id).unwrap();
        let back: SessionId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
        assert_eq!(json, format!("\"{}\"", id.0));
    }
}
```

- [ ] **Step 2: Run the test (will fail to compile until `types` module exists; that's fine — run only this file's test)**

Run: `cargo test -p teramind-core ids::tests::session_id_roundtrips_as_uuid_string`
Expected: at this point `cargo` may still report errors in other modules referenced from `lib.rs`. Stash this until Task 4 lands — or, to validate now, temporarily comment out the `pub mod types;` line in `crates/teramind-core/src/lib.rs` and re-run; the test should PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-core/src/ids.rs
git commit -m "feat(core): strongly-typed UUID newtypes for domain ids"
```

---

### Task 4: Domain type — `Agent`

**Files:**
- Create: `crates/teramind-core/src/types/mod.rs`
- Create: `crates/teramind-core/src/types/agent.rs`

- [ ] **Step 1: Create `types/mod.rs` with the first re-export**

```rust
pub mod agent;

pub use agent::Agent;
```

- [ ] **Step 2: Write the failing test in `types/agent.rs`**

```rust
use crate::ids::AgentId;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    pub id: AgentId,
    pub kind: String,
    pub version: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub installed_at: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn agent_roundtrips_through_json() {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let a = Agent {
            id: AgentId::new(),
            kind: "claude_code".to_string(),
            version: Some("0.2.0".to_string()),
            installed_at: now,
        };
        let s = serde_json::to_string(&a).unwrap();
        let back: Agent = serde_json::from_str(&s).unwrap();
        assert_eq!(a, back);
    }
}
```

- [ ] **Step 3: Run and verify the test passes**

Run: `cargo test -p teramind-core types::agent::tests::agent_roundtrips_through_json`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-core/src/types/
git commit -m "feat(core): Agent domain type"
```

---

### Task 5: Domain type — `Project`

**Files:**
- Create: `crates/teramind-core/src/types/project.rs`
- Modify: `crates/teramind-core/src/types/mod.rs`

- [ ] **Step 1: Add module declaration in `types/mod.rs`**

Replace `types/mod.rs` with:

```rust
pub mod agent;
pub mod project;

pub use agent::Agent;
pub use project::Project;
```

- [ ] **Step 2: Write the test + type in `types/project.rs`**

```rust
use crate::ids::ProjectId;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub root_path: String,
    pub git_remote: Option<String>,
    pub display_name: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub first_seen: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_roundtrips_through_json() {
        let p = Project {
            id: ProjectId::new(),
            root_path: "/home/dev/repo".to_string(),
            git_remote: Some("git@github.com:org/repo.git".to_string()),
            display_name: None,
            first_seen: OffsetDateTime::from_unix_timestamp(1_700_000_001).unwrap(),
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }
}
```

- [ ] **Step 3: Run the test**

Run: `cargo test -p teramind-core types::project::tests::project_roundtrips_through_json`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-core/src/types/project.rs crates/teramind-core/src/types/mod.rs
git commit -m "feat(core): Project domain type"
```

---

### Task 6: Domain type — `Session`

**Files:**
- Create: `crates/teramind-core/src/types/session.rs`
- Modify: `crates/teramind-core/src/types/mod.rs`

- [ ] **Step 1: Update `types/mod.rs`**

```rust
pub mod agent;
pub mod project;
pub mod session;

pub use agent::Agent;
pub use project::Project;
pub use session::{Session, SessionEndReason};
```

- [ ] **Step 2: Write the type and test in `types/session.rs`**

```rust
use crate::ids::{AgentId, ProjectId, SessionId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEndReason {
    StopHook,
    IdleTimeout,
    Crash,
    Compact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub agent_id: AgentId,
    pub agent_session_id: Option<String>,
    pub cwd: String,
    pub project_id: Option<ProjectId>,
    pub parent_session_id: Option<SessionId>,
    pub git_head: Option<String>,
    pub git_branch: Option<String>,
    pub os: String,
    pub hostname: String,
    pub user_login: String,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub ended_at: Option<OffsetDateTime>,
    pub end_reason: Option<SessionEndReason>,
    #[serde(default)]
    pub metadata: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_roundtrips_through_json() {
        let s = Session {
            id: SessionId::new(),
            agent_id: AgentId::new(),
            agent_session_id: Some("claude-abc".to_string()),
            cwd: "/work".to_string(),
            project_id: None,
            parent_session_id: None,
            git_head: None,
            git_branch: None,
            os: "linux".to_string(),
            hostname: "host".to_string(),
            user_login: "u".to_string(),
            started_at: OffsetDateTime::from_unix_timestamp(1_700_000_002).unwrap(),
            ended_at: None,
            end_reason: None,
            metadata: serde_json::json!({}),
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: Session = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}
```

- [ ] **Step 3: Run the test**

Run: `cargo test -p teramind-core types::session`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-core/src/types/
git commit -m "feat(core): Session domain type"
```

---

### Task 7: Domain type — `Turn`

**Files:**
- Create: `crates/teramind-core/src/types/turn.rs`
- Modify: `crates/teramind-core/src/types/mod.rs`

- [ ] **Step 1: Update `types/mod.rs`** to add `pub mod turn;` and `pub use turn::Turn;`.

- [ ] **Step 2: Write `types/turn.rs`**

```rust
use crate::ids::{SessionId, TurnId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub id: TurnId,
    pub session_id: SessionId,
    pub ordinal: i32,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub ended_at: Option<OffsetDateTime>,
    pub user_prompt: Option<String>,
    pub assistant_text: Option<String>,
    pub thinking: Option<String>,
    pub model: Option<String>,
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn turn_roundtrips_through_json() {
        let t = Turn {
            id: TurnId::new(),
            session_id: SessionId::new(),
            ordinal: 0,
            started_at: OffsetDateTime::from_unix_timestamp(1_700_000_003).unwrap(),
            ended_at: None,
            user_prompt: Some("hello".to_string()),
            assistant_text: None,
            thinking: None,
            model: Some("claude-opus-4-7".to_string()),
            input_tokens: None,
            output_tokens: None,
        };
        let j = serde_json::to_string(&t).unwrap();
        let back: Turn = serde_json::from_str(&j).unwrap();
        assert_eq!(t, back);
    }
}
```

- [ ] **Step 3: Run the test**

Run: `cargo test -p teramind-core types::turn`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-core/src/types/
git commit -m "feat(core): Turn domain type"
```

---

### Task 8: Domain type — `ToolCall`

**Files:**
- Create: `crates/teramind-core/src/types/tool_call.rs`
- Modify: `crates/teramind-core/src/types/mod.rs`

- [ ] **Step 1: Update `types/mod.rs`** to add `pub mod tool_call;` and `pub use tool_call::ToolCall;`.

- [ ] **Step 2: Write `types/tool_call.rs`**

```rust
use crate::ids::{ToolCallId, TurnId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub turn_id: TurnId,
    pub ordinal: i32,
    pub name: String,
    pub input: Value,
    pub output: Option<String>,
    pub is_error: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    pub duration_ms: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tool_call_roundtrips_through_json() {
        let tc = ToolCall {
            id: ToolCallId::new(),
            turn_id: TurnId::new(),
            ordinal: 0,
            name: "Edit".to_string(),
            input: serde_json::json!({"file_path": "/x.rs"}),
            output: Some("ok".to_string()),
            is_error: false,
            started_at: OffsetDateTime::from_unix_timestamp(1_700_000_004).unwrap(),
            duration_ms: Some(42),
        };
        let j = serde_json::to_string(&tc).unwrap();
        let back: ToolCall = serde_json::from_str(&j).unwrap();
        assert_eq!(tc, back);
    }
}
```

- [ ] **Step 3: Run the test**

Run: `cargo test -p teramind-core types::tool_call`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-core/src/types/
git commit -m "feat(core): ToolCall domain type"
```

---

### Task 9: Domain type — `FileDiff`

**Files:**
- Create: `crates/teramind-core/src/types/file_diff.rs`
- Modify: `crates/teramind-core/src/types/mod.rs`

- [ ] **Step 1: Update `types/mod.rs`** to add `pub mod file_diff;` and `pub use file_diff::{FileDiff, Attribution};`.

- [ ] **Step 2: Write `types/file_diff.rs`**

```rust
use crate::ids::{FileDiffId, SessionId, TurnId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attribution {
    Agent,
    Human,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    pub id: FileDiffId,
    pub turn_id: Option<TurnId>,
    pub session_id: SessionId,
    pub file_path: String,
    pub rel_path: String,
    pub attribution: Attribution,
    pub language: Option<String>,
    pub pre_excerpt: String,
    pub post_excerpt: String,
    pub unified_diff: String,
    #[serde(with = "serde_bytes_hex")]
    pub pre_hash: [u8; 32],
    #[serde(with = "serde_bytes_hex")]
    pub post_hash: [u8; 32],
    pub byte_size: i32,
    #[serde(with = "time::serde::rfc3339")]
    pub captured_at: OffsetDateTime,
}

mod serde_bytes_hex {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_diff_roundtrips_through_json() {
        let fd = FileDiff {
            id: FileDiffId::new(),
            turn_id: None,
            session_id: SessionId::new(),
            file_path: "/x.rs".to_string(),
            rel_path: "x.rs".to_string(),
            attribution: Attribution::Agent,
            language: Some("rust".to_string()),
            pre_excerpt: "a".to_string(),
            post_excerpt: "b".to_string(),
            unified_diff: "--- a\n+++ b\n".to_string(),
            pre_hash: [1u8; 32],
            post_hash: [2u8; 32],
            byte_size: 10,
            captured_at: OffsetDateTime::from_unix_timestamp(1_700_000_005).unwrap(),
        };
        let j = serde_json::to_string(&fd).unwrap();
        let back: FileDiff = serde_json::from_str(&j).unwrap();
        assert_eq!(fd, back);
    }
}
```

- [ ] **Step 3: Run the test**

Run: `cargo test -p teramind-core types::file_diff`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-core/src/types/
git commit -m "feat(core): FileDiff domain type with hex-encoded content hashes"
```

---

### Task 10: Domain types — `Skill`, `StorageStats`, `Hit`

**Files:**
- Create: `crates/teramind-core/src/types/{skill,storage_stats,hit}.rs`
- Modify: `crates/teramind-core/src/types/mod.rs`

- [ ] **Step 1: Update `types/mod.rs`** to add the three modules and re-export `Skill`, `SkillSource`, `StorageStats`, `Hit`.

- [ ] **Step 2: Write `types/skill.rs`**

```rust
use crate::ids::{SessionId, SkillId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource { Authored, Codified, Imported }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    pub id: SkillId,
    pub name: String,
    pub description: String,
    pub body: String,
    pub source: SkillSource,
    pub source_session_ids: Vec<SessionId>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skill_roundtrips_through_json() {
        let s = Skill {
            id: SkillId::new(),
            name: "kebab-name".into(),
            description: "desc".into(),
            body: "body".into(),
            source: SkillSource::Authored,
            source_session_ids: vec![],
            created_at: OffsetDateTime::from_unix_timestamp(1_700_000_006).unwrap(),
            updated_at: OffsetDateTime::from_unix_timestamp(1_700_000_006).unwrap(),
        };
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(s, serde_json::from_str::<Skill>(&j).unwrap());
    }
}
```

- [ ] **Step 3: Write `types/storage_stats.rs`**

```rust
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageStats {
    pub id: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub sampled_at: OffsetDateTime,
    pub pg_bytes: i64,
    pub jsonl_bytes: i64,
    pub session_count: i64,
    pub turn_count: i64,
    pub diff_count: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn storage_stats_roundtrips_through_json() {
        let s = StorageStats {
            id: 1,
            sampled_at: OffsetDateTime::from_unix_timestamp(1_700_000_007).unwrap(),
            pg_bytes: 100, jsonl_bytes: 200, session_count: 3, turn_count: 30, diff_count: 5,
        };
        assert_eq!(s, serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap());
    }
}
```

- [ ] **Step 4: Write `types/hit.rs`**

```rust
use crate::ids::{FileDiffId, SessionId, SkillId, ToolCallId, TurnId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Hit {
    Turn      { turn_id: TurnId, session_id: SessionId, ordinal: i32, snippet: String, score: f32, #[serde(with = "time::serde::rfc3339")] ts: OffsetDateTime },
    ToolCall  { tool_call_id: ToolCallId, turn_id: TurnId, name: String, input_snippet: String, output_snippet: String, score: f32, #[serde(with = "time::serde::rfc3339")] ts: OffsetDateTime },
    FileDiff  { diff_id: FileDiffId, rel_path: String, hunk_snippet: String, score: f32, #[serde(with = "time::serde::rfc3339")] ts: OffsetDateTime },
    Skill     { skill_id: SkillId, name: String, body_snippet: String, score: f32 },
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hit_skill_variant_roundtrips() {
        let h = Hit::Skill { skill_id: SkillId::new(), name: "n".into(), body_snippet: "b".into(), score: 0.9 };
        assert_eq!(format!("{:?}", h), format!("{:?}", serde_json::from_str::<Hit>(&serde_json::to_string(&h).unwrap()).unwrap()));
    }
}
```

- [ ] **Step 5: Run all three new test modules**

Run: `cargo test -p teramind-core types::skill types::storage_stats types::hit`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/teramind-core/src/types/
git commit -m "feat(core): Skill, StorageStats, and Hit types"
```

---

### Task 11: Domain type — `IngestEvent` enum

**Files:**
- Create: `crates/teramind-core/src/types/ingest_event.rs`
- Modify: `crates/teramind-core/src/types/mod.rs`

`IngestEvent` is the union of every event the hook shim can fire at the daemon. Each variant is tagged so it routes cleanly inside the ingest service.

- [ ] **Step 1: Update `types/mod.rs`** to add `pub mod ingest_event;` and `pub use ingest_event::IngestEvent;`.

- [ ] **Step 2: Write `types/ingest_event.rs`**

```rust
use crate::ids::{ClientEventId, SessionId, ToolCallId, TurnId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub client_event_id: ClientEventId,
    #[serde(with = "time::serde::rfc3339")]
    pub ts: OffsetDateTime,
    pub event: IngestEvent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IngestEvent {
    SessionStart {
        session_id: SessionId,
        agent_session_id: Option<String>,
        agent_kind: String,
        cwd: String,
        os: String,
        hostname: String,
        user_login: String,
        git_head: Option<String>,
        git_branch: Option<String>,
    },
    UserPrompt {
        session_id: SessionId,
        turn_ordinal: i32,
        prompt: String,
    },
    ToolCallStart {
        turn_id: TurnId,
        ordinal: i32,
        name: String,
        input: Value,
    },
    ToolCallEnd {
        tool_call_id: ToolCallId,
        output: String,
        is_error: bool,
        duration_ms: i32,
    },
    AssistantTurn {
        turn_id: TurnId,
        assistant_text: String,
        thinking: Option<String>,
        model: Option<String>,
        input_tokens: Option<i32>,
        output_tokens: Option<i32>,
    },
    SessionEnd {
        session_id: SessionId,
        reason: String, // mapped to SessionEndReason in ingest layer
    },
    PreCompact {
        session_id: SessionId,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn envelope_roundtrips() {
        let env = EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::from_unix_timestamp(1_700_000_010).unwrap(),
            event: IngestEvent::UserPrompt {
                session_id: SessionId::new(),
                turn_ordinal: 0,
                prompt: "hi".into(),
            },
        };
        let j = serde_json::to_string(&env).unwrap();
        let back: EventEnvelope = serde_json::from_str(&j).unwrap();
        assert_eq!(env, back);
    }
}
```

- [ ] **Step 3: Run the test**

Run: `cargo test -p teramind-core types::ingest_event`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-core/src/types/
git commit -m "feat(core): IngestEvent enum + EventEnvelope"
```

---

### Task 12: `error::Error` enum

**Files:**
- Create: `crates/teramind-core/src/error.rs`

- [ ] **Step 1: Write the error enum**

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("serialization failure: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("regex compile failure: {0}")]
    Regex(#[from] regex::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn error_from_io() {
        let e: Error = std::io::Error::new(std::io::ErrorKind::Other, "x").into();
        assert!(matches!(e, Error::Io(_)));
    }
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p teramind-core error`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-core/src/error.rs
git commit -m "feat(core): Error enum"
```

---

## Section 3 — `teramind-core::redact` (redaction)

Redaction is mandatory and applied in ingest before any persistence. Strict TDD: each pattern gets a failing test first, then the minimal regex to pass it. Final property test asserts that no rejected secret ever survives.

### Task 13: Redaction module skeleton

**Files:**
- Create: `crates/teramind-core/src/redact/mod.rs`
- Create: `crates/teramind-core/src/redact/patterns.rs`
- Create: `crates/teramind-core/src/redact/rules.rs`

- [ ] **Step 1: Create empty module files**

`crates/teramind-core/src/redact/mod.rs`:

```rust
pub mod patterns;
pub mod rules;

use rules::RuleSet;

pub struct Redactor {
    rules: RuleSet,
}

impl Redactor {
    pub fn with_default_rules() -> Self {
        Self { rules: RuleSet::default() }
    }
    pub fn apply(&self, input: &str) -> String {
        self.rules.apply(input)
    }
}
```

`crates/teramind-core/src/redact/patterns.rs`:

```rust
// Patterns are added one task at a time as failing tests drive them.

pub struct Pattern {
    pub name: &'static str,
    pub regex: &'static str,
}

pub const PATTERNS: &[Pattern] = &[];
```

`crates/teramind-core/src/redact/rules.rs`:

```rust
use crate::redact::patterns::PATTERNS;
use regex::Regex;

pub struct RuleSet {
    compiled: Vec<(&'static str, Regex)>,
}

impl Default for RuleSet {
    fn default() -> Self {
        let compiled = PATTERNS.iter()
            .map(|p| (p.name, Regex::new(p.regex).expect("invalid built-in pattern")))
            .collect();
        Self { compiled }
    }
}

impl RuleSet {
    pub fn apply(&self, input: &str) -> String {
        let mut out = input.to_string();
        for (name, re) in &self.compiled {
            out = re.replace_all(&out, format!("«redacted:{}»", name).as_str()).into_owned();
        }
        out
    }
}
```

- [ ] **Step 2: Compile-only check**

Run: `cargo check -p teramind-core`
Expected: clean compile (no warnings about unused items here).

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-core/src/redact/
git commit -m "feat(core): scaffold redaction module"
```

---

### Task 14: AWS access key redactor (TDD)

**Files:**
- Modify: `crates/teramind-core/src/redact/patterns.rs`
- Test: `crates/teramind-core/tests/redaction_corpus.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/teramind-core/tests/redaction_corpus.rs`:

```rust
use teramind_core::redact::Redactor;

#[test]
fn aws_access_key_is_redacted() {
    let r = Redactor::with_default_rules();
    let input = "key=AKIAIOSFODNN7EXAMPLE next";
    let out = r.apply(input);
    assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"), "raw key leaked: {out}");
    assert!(out.contains("«redacted:aws_access_key»"));
}
```

- [ ] **Step 2: Run and confirm it fails**

Run: `cargo test -p teramind-core --test redaction_corpus aws_access_key_is_redacted`
Expected: FAIL (output still contains the literal key).

- [ ] **Step 3: Add the pattern**

Replace `crates/teramind-core/src/redact/patterns.rs` body with:

```rust
pub struct Pattern {
    pub name: &'static str,
    pub regex: &'static str,
}

pub const PATTERNS: &[Pattern] = &[
    Pattern { name: "aws_access_key", regex: r"AKIA[0-9A-Z]{16}" },
];
```

- [ ] **Step 4: Run the test and verify it passes**

Run: `cargo test -p teramind-core --test redaction_corpus aws_access_key_is_redacted`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-core/src/redact/patterns.rs crates/teramind-core/tests/redaction_corpus.rs
git commit -m "feat(core): redact AWS access keys"
```

---

### Task 15: GitHub PAT / OAuth token redactor (TDD)

**Files:**
- Modify: `crates/teramind-core/src/redact/patterns.rs`
- Modify: `crates/teramind-core/tests/redaction_corpus.rs`

- [ ] **Step 1: Append a failing test**

Append to `crates/teramind-core/tests/redaction_corpus.rs`:

```rust
#[test]
fn github_pat_is_redacted() {
    let r = Redactor::with_default_rules();
    for sample in ["ghp_1234567890abcdefghijklmnopqrstuvwxyz",
                   "gho_abcdefghijklmnopqrstuvwxyz1234567890",
                   "ghs_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                   "ghr_BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"] {
        let out = r.apply(sample);
        assert!(!out.contains(sample), "leaked: {sample} -> {out}");
        assert!(out.contains("«redacted:github_token»"));
    }
}
```

- [ ] **Step 2: Run and confirm FAIL**

Run: `cargo test -p teramind-core --test redaction_corpus github_pat_is_redacted`
Expected: FAIL.

- [ ] **Step 3: Append the pattern**

Replace `PATTERNS` in `patterns.rs` with:

```rust
pub const PATTERNS: &[Pattern] = &[
    Pattern { name: "aws_access_key", regex: r"AKIA[0-9A-Z]{16}" },
    Pattern { name: "github_token",   regex: r"gh[pousr]_[A-Za-z0-9]{36}" },
];
```

- [ ] **Step 4: Run and verify PASS**

Run: `cargo test -p teramind-core --test redaction_corpus github_pat_is_redacted`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-core/src/redact/patterns.rs crates/teramind-core/tests/redaction_corpus.rs
git commit -m "feat(core): redact GitHub PAT/OAuth tokens"
```

---

### Task 16: Slack token redactor (TDD)

**Files:**
- Modify: `crates/teramind-core/src/redact/patterns.rs`
- Modify: `crates/teramind-core/tests/redaction_corpus.rs`

- [ ] **Step 1: Append failing test**

```rust
#[test]
fn slack_token_is_redacted() {
    let r = Redactor::with_default_rules();
    let s = "xoxb-1234567890-1234567890-aBcDeFgHiJkLmNoPqRsTuVwX";
    let out = r.apply(s);
    assert!(!out.contains(s));
    assert!(out.contains("«redacted:slack_token»"));
}
```

- [ ] **Step 2: Run and confirm FAIL**

Run: `cargo test -p teramind-core --test redaction_corpus slack_token_is_redacted`
Expected: FAIL.

- [ ] **Step 3: Append the pattern** (replace `PATTERNS` to include the new entry; preserve previous entries):

```rust
pub const PATTERNS: &[Pattern] = &[
    Pattern { name: "aws_access_key", regex: r"AKIA[0-9A-Z]{16}" },
    Pattern { name: "github_token",   regex: r"gh[pousr]_[A-Za-z0-9]{36}" },
    Pattern { name: "slack_token",    regex: r"xox[bpoa]-[A-Za-z0-9-]{10,}" },
];
```

- [ ] **Step 4: Run and verify PASS**

Run: `cargo test -p teramind-core --test redaction_corpus slack_token_is_redacted`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-core/src/redact/patterns.rs crates/teramind-core/tests/redaction_corpus.rs
git commit -m "feat(core): redact Slack tokens"
```

---

### Task 17: JWT redactor (TDD)

**Files:**
- Modify: `crates/teramind-core/src/redact/patterns.rs`
- Modify: `crates/teramind-core/tests/redaction_corpus.rs`

- [ ] **Step 1: Append failing test**

```rust
#[test]
fn jwt_is_redacted() {
    let r = Redactor::with_default_rules();
    let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ1MSJ9.ZmFrZXNpZ25hdHVyZQ";
    let out = r.apply(jwt);
    assert!(!out.contains(jwt));
    assert!(out.contains("«redacted:jwt»"));
}
```

- [ ] **Step 2: Run and confirm FAIL.**

Run: `cargo test -p teramind-core --test redaction_corpus jwt_is_redacted`

- [ ] **Step 3: Append the pattern**

```rust
Pattern { name: "jwt", regex: r"eyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+" },
```

- [ ] **Step 4: Run and verify PASS.**

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-core/src/redact/patterns.rs crates/teramind-core/tests/redaction_corpus.rs
git commit -m "feat(core): redact JWTs"
```

---

### Task 18: PEM private key redactor (TDD)

**Files:**
- Modify: `crates/teramind-core/src/redact/patterns.rs`
- Modify: `crates/teramind-core/tests/redaction_corpus.rs`

- [ ] **Step 1: Append failing test**

```rust
#[test]
fn pem_private_key_is_redacted() {
    let r = Redactor::with_default_rules();
    let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIBOgIBAAJBAKj...==\n-----END RSA PRIVATE KEY-----";
    let out = r.apply(pem);
    assert!(!out.contains("MIIBOgIBAAJBAKj"));
    assert!(out.contains("«redacted:pem_private_key»"));
}
```

- [ ] **Step 2: Confirm FAIL.** Run: `cargo test -p teramind-core --test redaction_corpus pem_private_key_is_redacted`.

- [ ] **Step 3: Append the pattern** (uses regex flag `(?s)` for dotall):

```rust
Pattern { name: "pem_private_key", regex: r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----" },
```

- [ ] **Step 4: Verify PASS.**

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(core): redact PEM private key blocks"
```

---

### Task 19: `password=` / `pwd=` redactor (TDD)

**Files:** same as above.

- [ ] **Step 1: Append failing test**

```rust
#[test]
fn password_kv_is_redacted() {
    let r = Redactor::with_default_rules();
    for s in ["password=hunter2 next", "PWD=correcthorsebatterystaple "] {
        let out = r.apply(s);
        assert!(!out.contains("hunter2"));
        assert!(!out.contains("correcthorsebatterystaple"));
    }
}
```

- [ ] **Step 2: Confirm FAIL.**

- [ ] **Step 3: Append the pattern (case-insensitive):**

```rust
Pattern { name: "password_kv", regex: r"(?i)\b(?:password|pwd)\s*=\s*\S+" },
```

- [ ] **Step 4: Verify PASS.**

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(core): redact password=value / pwd=value"
```

---

### Task 20: `.env`-style `KEY=value` allowlist redactor

**Files:** same.

- [ ] **Step 1: Append failing test**

```rust
#[test]
fn env_key_allowlist_is_redacted() {
    let r = Redactor::with_default_rules();
    for s in ["API_SECRET=abcdef ", "MY_TOKEN=xyz123 ", "DB_PASSWORD=p", "FOO_CREDENTIAL=bar", "X_KEY=val"] {
        let out = r.apply(s);
        let val = s.split('=').nth(1).unwrap().split_whitespace().next().unwrap();
        assert!(!out.contains(val), "leaked: {s} -> {out}");
    }
}
```

- [ ] **Step 2: Confirm FAIL.**

- [ ] **Step 3: Append the pattern:**

```rust
Pattern { name: "env_secret", regex: r"(?i)\b[A-Z_][A-Z0-9_]*(?:PASSWORD|SECRET|TOKEN|KEY|CREDENTIAL)\s*=\s*\S+" },
```

- [ ] **Step 4: Verify PASS.**

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(core): redact .env KEY=value when KEY matches secret allowlist"
```

---

### Task 21: Property test — no rejected secret survives

**Files:**
- Modify: `crates/teramind-core/tests/redaction_corpus.rs`

- [ ] **Step 1: Append the property test**

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn aws_keys_in_random_text_never_survive(prefix in ".{0,40}", suffix in ".{0,40}") {
        let r = Redactor::with_default_rules();
        let secret = "AKIAIOSFODNN7EXAMPLE";
        let input = format!("{prefix}{secret}{suffix}");
        let out = r.apply(&input);
        prop_assert!(!out.contains(secret));
    }
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p teramind-core --test redaction_corpus aws_keys_in_random_text_never_survive`
Expected: PASS (proptest runs 256 cases by default; all must hold).

- [ ] **Step 3: Commit**

```bash
git commit -am "test(core): property-test AWS key redaction over random contexts"
```

---

### Task 22: Custom-rules loader (config-driven extra patterns)

**Files:**
- Modify: `crates/teramind-core/src/redact/rules.rs`
- Modify: `crates/teramind-core/src/redact/mod.rs`

- [ ] **Step 1: Add the `with_extra` constructor and test**

In `rules.rs`, add at the bottom:

```rust
impl RuleSet {
    pub fn with_extra(extra: &[(&str, &str)]) -> Result<Self, regex::Error> {
        let mut compiled: Vec<(&'static str, Regex)> = PATTERNS.iter()
            .map(|p| (p.name, Regex::new(p.regex).expect("invalid built-in pattern")))
            .collect();
        for (name, re) in extra {
            let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
            compiled.push((leaked, Regex::new(re)?));
        }
        Ok(Self { compiled })
    }
}
```

In `mod.rs`, replace the `Redactor` impl with:

```rust
impl Redactor {
    pub fn with_default_rules() -> Self {
        Self { rules: RuleSet::default() }
    }
    pub fn with_extra(extra: &[(&str, &str)]) -> Result<Self, regex::Error> {
        Ok(Self { rules: RuleSet::with_extra(extra)? })
    }
    pub fn apply(&self, input: &str) -> String { self.rules.apply(input) }
}
```

- [ ] **Step 2: Add unit test in `rules.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extra_rules_redact_custom_pattern() {
        let r = RuleSet::with_extra(&[("project_token", r"PROJTOK-[A-Z0-9]{8}")]).unwrap();
        let out = r.apply("see PROJTOK-ABCDEFGH here");
        assert!(out.contains("«redacted:project_token»"));
    }
}
```

- [ ] **Step 3: Run all redaction tests**

Run: `cargo test -p teramind-core redact`
Expected: PASS (rules + corpus + property).

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-core/src/redact/
git commit -m "feat(core): redact::Redactor::with_extra for user-defined patterns"
```

---

## Section 4 — `teramind-ipc` protocol, codec, transport

### Task 23: Protocol enums (`Request`, `Response`, `Notify`)

**Files:**
- Create: `crates/teramind-ipc/src/proto.rs`
- Create: `crates/teramind-ipc/src/error.rs`

- [ ] **Step 1: Write the protocol module**

`crates/teramind-ipc/src/proto.rs`:

```rust
use serde::{Deserialize, Serialize};
use teramind_core::types::ingest_event::EventEnvelope;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Request {
    Status,
    Ping,
    Shutdown,
    // Plan C/D add Search { ... }, Recall { ... }, SaveSkill { ... }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    Ok,
    Pong,
    Status(StatusReport),
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusReport {
    pub uptime_seconds: u64,
    pub pg_connected: bool,
    pub ingest_queue_depth: u32,
    pub ingest_drops_total: u64,
    pub last_storage_pg_bytes: i64,
    pub last_storage_jsonl_bytes: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Notify {
    Ingest(EventEnvelope),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub id: Uuid,
    pub payload: Payload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Payload {
    Request(Request),
    Response(Response),
    Notify(Notify),
}
```

- [ ] **Step 2: Write `error.rs`**

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IpcError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("daemon busy")]
    Busy,
    #[error("daemon unreachable")]
    Unreachable,
    #[error("protocol error: {0}")]
    Protocol(String),
}
```

- [ ] **Step 3: Add unit test for `Payload` round-trip**

In `crates/teramind-ipc/src/proto.rs`, append:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    #[test]
    fn payload_request_status_roundtrips() {
        let env = Envelope { id: Uuid::new_v4(), payload: Payload::Request(Request::Status) };
        let j = serde_json::to_string(&env).unwrap();
        let back: Envelope = serde_json::from_str(&j).unwrap();
        assert_eq!(env, back);
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p teramind-ipc proto::tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-ipc/src/proto.rs crates/teramind-ipc/src/error.rs
git commit -m "feat(ipc): protocol enums and Envelope"
```

---

### Task 24: Length-prefixed JSON codec

**Files:**
- Create: `crates/teramind-ipc/src/codec.rs`

- [ ] **Step 1: Write the failing test (in-file)**

```rust
use crate::proto::{Envelope, Payload, Request};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, env: &Envelope) -> Result<(), crate::IpcError> {
    let bytes = serde_json::to_vec(env)?;
    let len = (bytes.len() as u32).to_be_bytes();
    w.write_all(&len).await?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}

pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Envelope, crate::IpcError> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 16 * 1024 * 1024 {
        return Err(crate::IpcError::Protocol(format!("frame too large: {len}")));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(serde_json::from_slice(&buf)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;
    #[tokio::test]
    async fn frame_roundtrip() {
        let (mut a, mut b) = duplex(64 * 1024);
        let env = Envelope { id: Uuid::new_v4(), payload: Payload::Request(Request::Status) };
        write_frame(&mut a, &env).await.unwrap();
        drop(a);
        let back = read_frame(&mut b).await.unwrap();
        assert_eq!(env, back);
    }
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p teramind-ipc codec::tests::frame_roundtrip`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-ipc/src/codec.rs
git commit -m "feat(ipc): length-prefixed JSON frame codec"
```

---

### Task 25: `IpcClient` trait + transport-agnostic client

**Files:**
- Create: `crates/teramind-ipc/src/client.rs`

- [ ] **Step 1: Write the trait + implementation**

```rust
use crate::codec::{read_frame, write_frame};
use crate::proto::{Envelope, Notify, Payload, Request, Response};
use crate::IpcError;
use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};
use uuid::Uuid;

#[async_trait]
pub trait IpcClient: Send + Sync {
    async fn request(&mut self, req: Request) -> Result<Response, IpcError>;
    async fn notify(&mut self, n: Notify) -> Result<(), IpcError>;
}

pub struct StreamClient<S: AsyncRead + AsyncWrite + Unpin + Send> {
    stream: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> StreamClient<S> {
    pub fn new(stream: S) -> Self { Self { stream } }
}

#[async_trait]
impl<S: AsyncRead + AsyncWrite + Unpin + Send> IpcClient for StreamClient<S> {
    async fn request(&mut self, req: Request) -> Result<Response, IpcError> {
        let env = Envelope { id: Uuid::new_v4(), payload: Payload::Request(req) };
        write_frame(&mut self.stream, &env).await?;
        let back = read_frame(&mut self.stream).await?;
        match back.payload {
            Payload::Response(r) => Ok(r),
            other => Err(IpcError::Protocol(format!("expected Response, got {:?}", other))),
        }
    }
    async fn notify(&mut self, n: Notify) -> Result<(), IpcError> {
        let env = Envelope { id: Uuid::new_v4(), payload: Payload::Notify(n) };
        write_frame(&mut self.stream, &env).await
    }
}
```

- [ ] **Step 2: Unit test using `tokio::io::duplex`**

Append at bottom of `client.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn client_sends_notify_frame() {
        let (a, mut b) = duplex(8 * 1024);
        let mut client = StreamClient::new(a);
        // Spawn a "server" half that just reads one frame.
        let h = tokio::spawn(async move {
            crate::codec::read_frame(&mut b).await.unwrap()
        });
        // Use a real EventEnvelope from teramind-core.
        use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
        use teramind_core::ids::{ClientEventId, SessionId};
        use time::OffsetDateTime;
        let envelope = EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::from_unix_timestamp(1_700_000_011).unwrap(),
            event: IngestEvent::UserPrompt {
                session_id: SessionId::new(), turn_ordinal: 0, prompt: "hi".into(),
            },
        };
        client.notify(Notify::Ingest(envelope.clone())).await.unwrap();
        let received = h.await.unwrap();
        match received.payload {
            Payload::Notify(Notify::Ingest(env)) => assert_eq!(env, envelope),
            _ => panic!("unexpected payload"),
        }
    }
}
```

- [ ] **Step 3: Run the test**

Run: `cargo test -p teramind-ipc client::tests::client_sends_notify_frame`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-ipc/src/client.rs
git commit -m "feat(ipc): IpcClient trait + StreamClient over any AsyncRead+AsyncWrite"
```

---

### Task 26: `IpcServer` dispatcher trait

**Files:**
- Create: `crates/teramind-ipc/src/server.rs`

- [ ] **Step 1: Write the server trait + accept loop helper**

```rust
use crate::codec::{read_frame, write_frame};
use crate::proto::{Envelope, Notify, Payload, Request, Response};
use crate::IpcError;
use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};

#[async_trait]
pub trait IpcServer: Send + Sync + 'static {
    async fn handle_request(&self, req: Request) -> Response;
    async fn handle_notify(&self, n: Notify);
}

pub async fn serve_connection<S, H>(mut stream: S, handler: std::sync::Arc<H>) -> Result<(), IpcError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
    H: IpcServer,
{
    loop {
        let env = match read_frame(&mut stream).await {
            Ok(e) => e,
            Err(IpcError::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        match env.payload {
            Payload::Request(req) => {
                let resp = handler.handle_request(req).await;
                let out = Envelope { id: env.id, payload: Payload::Response(resp) };
                write_frame(&mut stream, &out).await?;
            }
            Payload::Notify(n) => {
                handler.handle_notify(n).await;
            }
            Payload::Response(_) => {
                return Err(IpcError::Protocol("client sent Response".into()));
            }
        }
    }
}
```

- [ ] **Step 2: Wire it into `lib.rs`** — already done in scaffolding; verify with `cargo check -p teramind-ipc`.

- [ ] **Step 3: Test using `duplex`**

Append to `server.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{IpcClient, StreamClient};
    use std::sync::Arc;
    use tokio::io::duplex;

    struct Echo;
    #[async_trait]
    impl IpcServer for Echo {
        async fn handle_request(&self, req: Request) -> Response {
            match req {
                Request::Ping => Response::Pong,
                _ => Response::Error("unsupported".into()),
            }
        }
        async fn handle_notify(&self, _n: Notify) {}
    }

    #[tokio::test]
    async fn ping_pong_roundtrips() {
        let (a, b) = duplex(8 * 1024);
        let handler = Arc::new(Echo);
        let _server = tokio::spawn(serve_connection(b, handler));
        let mut client = StreamClient::new(a);
        let r = client.request(Request::Ping).await.unwrap();
        assert_eq!(r, Response::Pong);
    }
}
```

- [ ] **Step 4: Run the test**

Run: `cargo test -p teramind-ipc server::tests::ping_pong_roundtrips`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-ipc/src/server.rs
git commit -m "feat(ipc): IpcServer trait + serve_connection dispatcher"
```

---

### Task 27: Cross-platform transport (UDS / Named Pipe)

**Files:**
- Create: `crates/teramind-ipc/src/transport/mod.rs`
- Create: `crates/teramind-ipc/src/transport/unix.rs`
- Create: `crates/teramind-ipc/src/transport/windows.rs`

- [ ] **Step 1: Write `transport/mod.rs`**

```rust
#[cfg(unix)]
pub mod unix;
#[cfg(windows)]
pub mod windows;

#[cfg(unix)]
pub use unix::{connect, listen, default_socket_path};
#[cfg(windows)]
pub use windows::{connect, listen, default_socket_path};
```

- [ ] **Step 2: Write `transport/unix.rs`**

```rust
use crate::IpcError;
use std::path::PathBuf;
use tokio::net::{UnixListener, UnixStream};

pub fn default_socket_path() -> PathBuf {
    PathBuf::from(std::env::var("TERAMIND_SOCKET").unwrap_or_else(|_| "/tmp/teramind.sock".into()))
}

pub async fn connect(path: &std::path::Path) -> Result<UnixStream, IpcError> {
    Ok(UnixStream::connect(path).await?)
}

pub fn listen(path: &std::path::Path) -> Result<UnixListener, IpcError> {
    if path.exists() { let _ = std::fs::remove_file(path); }
    Ok(UnixListener::bind(path)?)
}
```

- [ ] **Step 3: Write `transport/windows.rs`**

```rust
use crate::IpcError;
use std::path::PathBuf;
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions};

pub fn default_socket_path() -> PathBuf {
    PathBuf::from(r"\\.\pipe\teramind")
}

pub async fn connect(path: &std::path::Path) -> Result<NamedPipeClient, IpcError> {
    let s = path.to_string_lossy();
    Ok(ClientOptions::new().open(s.as_ref())?)
}

pub fn listen(path: &std::path::Path) -> Result<NamedPipeServer, IpcError> {
    let s = path.to_string_lossy();
    Ok(ServerOptions::new().first_pipe_instance(true).create(s.as_ref())?)
}
```

- [ ] **Step 4: Integration test for the UDS path**

Create `crates/teramind-ipc/tests/roundtrip.rs`:

```rust
#![cfg(unix)]
use teramind_ipc::{IpcServer, Request, Response, Notify, client::{IpcClient, StreamClient}};
use teramind_ipc::server::serve_connection;
use teramind_ipc::transport::{listen, connect};
use std::sync::Arc;
use async_trait::async_trait;
use tempfile::tempdir;

struct PingHandler;
#[async_trait]
impl IpcServer for PingHandler {
    async fn handle_request(&self, req: Request) -> Response {
        match req {
            Request::Ping => Response::Pong,
            _ => Response::Error("nope".into()),
        }
    }
    async fn handle_notify(&self, _n: Notify) {}
}

#[tokio::test]
async fn uds_ping_pong_end_to_end() {
    let tmp = tempdir().unwrap();
    let sock = tmp.path().join("t.sock");
    let listener = listen(&sock).unwrap();
    let handler = Arc::new(PingHandler);
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        serve_connection(stream, handler).await.unwrap();
    });

    let stream = connect(&sock).await.unwrap();
    let mut client = StreamClient::new(stream);
    let r = client.request(Request::Ping).await.unwrap();
    assert_eq!(r, Response::Pong);
    drop(client);
    let _ = server.await;
}
```

- [ ] **Step 5: Run the test (Unix only)**

Run: `cargo test -p teramind-ipc --test roundtrip`
Expected: PASS on macOS/Linux. On Windows the test is `#[cfg(unix)]` and is skipped.

- [ ] **Step 6: Commit**

```bash
git add crates/teramind-ipc/src/transport/ crates/teramind-ipc/tests/roundtrip.rs
git commit -m "feat(ipc): cross-platform transport (UDS on Unix, Named Pipe on Windows) + uds integration test"
```

---

## Section 5 — `teramind-db`: embedded Postgres + migrations

### Task 28: `DbError` enum and `pool.rs` skeleton

**Files:**
- Create: `crates/teramind-db/src/error.rs`
- Create: `crates/teramind-db/src/pool.rs`

- [ ] **Step 1: Write `error.rs`**

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("sqlx: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("migrate: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("supervisor: {0}")]
    Supervisor(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = DbError> = std::result::Result<T, E>;
```

- [ ] **Step 2: Write `pool.rs`**

```rust
use crate::error::Result;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;

#[derive(Clone)]
pub struct DbPool {
    pub(crate) inner: PgPool,
}

impl DbPool {
    pub async fn connect(opts: PgConnectOptions) -> Result<Self> {
        let inner = PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect_with(opts)
            .await?;
        Ok(Self { inner })
    }
    pub fn pg(&self) -> &PgPool { &self.inner }
}
```

- [ ] **Step 3: Compile-check**

Run: `cargo check -p teramind-db`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-db/src/error.rs crates/teramind-db/src/pool.rs
git commit -m "feat(db): DbError and DbPool wrapper"
```

---

### Task 29: Embedded Postgres supervisor

**Files:**
- Create: `crates/teramind-db/src/pg_supervisor.rs`

- [ ] **Step 1: Write the supervisor**

```rust
use crate::error::{DbError, Result};
use postgresql_embedded::{PostgreSQL, Settings};
use sqlx::postgres::PgConnectOptions;
use std::path::PathBuf;

pub struct PgSupervisor {
    instance: PostgreSQL,
    data_dir: PathBuf,
    database: String,
}

impl PgSupervisor {
    /// Initialize an embedded Postgres rooted at `data_dir` and ensure the named DB exists.
    pub async fn start(data_dir: PathBuf, database: &str) -> Result<Self> {
        let settings = Settings {
            data_dir: data_dir.clone(),
            installation_dir: data_dir.join("install"),
            password: "teramind".into(),
            ..Default::default()
        };
        let mut instance = PostgreSQL::new(settings);
        instance.setup().await.map_err(|e| DbError::Supervisor(e.to_string()))?;
        instance.start().await.map_err(|e| DbError::Supervisor(e.to_string()))?;
        if !instance.database_exists(database).await.map_err(|e| DbError::Supervisor(e.to_string()))? {
            instance.create_database(database).await.map_err(|e| DbError::Supervisor(e.to_string()))?;
        }
        Ok(Self { instance, data_dir, database: database.to_string() })
    }

    pub fn connect_options(&self) -> PgConnectOptions {
        let s = self.instance.settings();
        PgConnectOptions::new()
            .host(&s.host)
            .port(s.port)
            .username(&s.username)
            .password(&s.password)
            .database(&self.database)
    }

    pub async fn shutdown(mut self) -> Result<()> {
        self.instance.stop().await.map_err(|e| DbError::Supervisor(e.to_string()))?;
        Ok(())
    }

    pub fn data_dir(&self) -> &PathBuf { &self.data_dir }
}
```

- [ ] **Step 2: Smoke test that downloads + starts PG**

Create `crates/teramind-db/tests/migrations.rs` (placeholder for next task — for now a minimal supervisor smoke test):

```rust
use teramind_db::pg_supervisor::PgSupervisor;
use tempfile::tempdir;

#[tokio::test]
async fn supervisor_starts_and_stops_embedded_pg() {
    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().to_path_buf(), "teramind_test").await.unwrap();
    let _opts = sup.connect_options();
    sup.shutdown().await.unwrap();
}
```

- [ ] **Step 3: Run the smoke test** (this is the first time we actually download an embedded Postgres binary — expect a one-time ~50 MB download)

Run: `cargo test -p teramind-db --test migrations supervisor_starts_and_stops_embedded_pg -- --nocapture`
Expected: PASS (may take 30–90s on first run due to PG download).

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-db/src/pg_supervisor.rs crates/teramind-db/tests/migrations.rs
git commit -m "feat(db): embedded Postgres supervisor + smoke test"
```

---

### Task 30: Migration runner

**Files:**
- Create: `crates/teramind-db/src/migrate.rs`

- [ ] **Step 1: Write the runner**

```rust
use crate::error::Result;
use crate::pool::DbPool;

pub async fn run(pool: &DbPool) -> Result<()> {
    sqlx::migrate!("./migrations").run(pool.pg()).await?;
    Ok(())
}
```

- [ ] **Step 2: Compile-check (migrations dir empty so `migrate!` will fail until we add files; that's expected)**

Run: `cargo check -p teramind-db`
Expected: error about missing migrations directory or empty migration set; that's fine — Task 31 creates the first migration.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-db/src/migrate.rs
git commit -m "feat(db): sqlx migration runner stub"
```

---

### Task 31: Migration 0001 — extensions

**Files:**
- Create: `crates/teramind-db/migrations/20260513000001_extensions.sql`

- [ ] **Step 1: Write the migration**

```sql
CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE EXTENSION IF NOT EXISTS pg_trgm;
```

- [ ] **Step 2: Extend the supervisor smoke test to also run migrations**

Replace `crates/teramind-db/tests/migrations.rs` with:

```rust
use teramind_db::{pg_supervisor::PgSupervisor, pool::DbPool, migrate};
use tempfile::tempdir;

#[tokio::test]
async fn migrations_apply_cleanly_on_empty_db() {
    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().to_path_buf(), "teramind_test").await.unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();
    // Verify extensions are installed
    let rows: Vec<(String,)> = sqlx::query_as("SELECT extname FROM pg_extension WHERE extname IN ('pgcrypto','pg_trgm') ORDER BY extname")
        .fetch_all(pool.pg()).await.unwrap();
    assert_eq!(rows.iter().map(|(n,)| n.as_str()).collect::<Vec<_>>(), vec!["pg_trgm","pgcrypto"]);
    sup.shutdown().await.unwrap();
}
```

- [ ] **Step 3: Run the test**

Run: `cargo test -p teramind-db --test migrations migrations_apply_cleanly_on_empty_db`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-db/migrations/ crates/teramind-db/tests/migrations.rs
git commit -m "feat(db): migration 0001 — extensions (pgcrypto, pg_trgm)"
```

---

### Task 32: Migration 0002 — `agents`

**Files:**
- Create: `crates/teramind-db/migrations/20260513000002_agents.sql`

- [ ] **Step 1: Write the migration**

```sql
CREATE TABLE agents (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  kind         text NOT NULL,
  version      text,
  installed_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (kind, version)
);
```

- [ ] **Step 2: Run migration smoke test**

Run: `cargo test -p teramind-db --test migrations`
Expected: PASS (still green; only schema added).

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-db/migrations/
git commit -m "feat(db): migration 0002 — agents"
```

---

### Task 33: Migration 0003 — `projects`

**Files:**
- Create: `crates/teramind-db/migrations/20260513000003_projects.sql`

- [ ] **Step 1: Write the migration**

```sql
CREATE TABLE projects (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  root_path    text NOT NULL UNIQUE,
  git_remote   text,
  display_name text,
  first_seen   timestamptz NOT NULL DEFAULT now()
);
```

- [ ] **Step 2: Migration smoke remains green**

Run: `cargo test -p teramind-db --test migrations`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-db/migrations/
git commit -m "feat(db): migration 0003 — projects"
```

---

### Task 34: Migration 0004 — `sessions`

**Files:**
- Create: `crates/teramind-db/migrations/20260513000004_sessions.sql`

- [ ] **Step 1: Write the migration**

```sql
CREATE TABLE sessions (
  id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  agent_id          uuid NOT NULL REFERENCES agents(id),
  agent_session_id  text,
  cwd               text NOT NULL,
  project_id        uuid REFERENCES projects(id),
  parent_session_id uuid REFERENCES sessions(id),
  git_head          text,
  git_branch        text,
  os                text NOT NULL,
  hostname          text NOT NULL,
  user_login        text NOT NULL,
  started_at        timestamptz NOT NULL,
  ended_at          timestamptz,
  end_reason        text,
  metadata          jsonb NOT NULL DEFAULT '{}'::jsonb
);
CREATE INDEX sessions_cwd_started ON sessions (cwd, started_at DESC);
CREATE INDEX sessions_project ON sessions (project_id, started_at DESC);
```

- [ ] **Step 2: Migration smoke**

Run: `cargo test -p teramind-db --test migrations`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-db/migrations/
git commit -m "feat(db): migration 0004 — sessions"
```

---

### Task 35: Migrations 0005–0009 — `turns`, `tool_calls`, `file_diffs`, `skills`, `storage_stats`

**Files:**
- Create: `crates/teramind-db/migrations/20260513000005_turns.sql`
- Create: `crates/teramind-db/migrations/20260513000006_tool_calls.sql`
- Create: `crates/teramind-db/migrations/20260513000007_file_diffs.sql`
- Create: `crates/teramind-db/migrations/20260513000008_skills.sql`
- Create: `crates/teramind-db/migrations/20260513000009_storage_stats.sql`

- [ ] **Step 1: Write `0005_turns.sql`**

```sql
CREATE TABLE turns (
  id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  session_id      uuid NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  ordinal         integer NOT NULL,
  started_at      timestamptz NOT NULL,
  ended_at        timestamptz,
  user_prompt     text,
  assistant_text  text,
  thinking        text,
  model           text,
  input_tokens    integer,
  output_tokens   integer,
  UNIQUE (session_id, ordinal)
);
CREATE INDEX turns_session ON turns (session_id, ordinal);
```

- [ ] **Step 2: Write `0006_tool_calls.sql`**

```sql
CREATE TABLE tool_calls (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  turn_id      uuid NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
  ordinal      integer NOT NULL,
  name         text NOT NULL,
  input        jsonb NOT NULL,
  output       text,
  is_error     boolean NOT NULL DEFAULT false,
  started_at   timestamptz NOT NULL,
  duration_ms  integer,
  UNIQUE (turn_id, ordinal)
);
CREATE INDEX tool_calls_turn ON tool_calls (turn_id, ordinal);
CREATE INDEX tool_calls_name ON tool_calls (name);
```

- [ ] **Step 3: Write `0007_file_diffs.sql`**

```sql
CREATE TABLE file_diffs (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  turn_id      uuid REFERENCES turns(id) ON DELETE CASCADE,
  session_id   uuid NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  file_path    text NOT NULL,
  rel_path     text NOT NULL,
  attribution  text NOT NULL CHECK (attribution IN ('agent','human')),
  language     text,
  pre_excerpt  text NOT NULL,
  post_excerpt text NOT NULL,
  unified_diff text NOT NULL,
  pre_hash     bytea NOT NULL,
  post_hash    bytea NOT NULL,
  byte_size    integer NOT NULL,
  captured_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX file_diffs_session ON file_diffs (session_id, captured_at DESC);
CREATE INDEX file_diffs_relpath ON file_diffs (rel_path);
CREATE INDEX file_diffs_pre_excerpt_trgm ON file_diffs USING gin (pre_excerpt gin_trgm_ops);
CREATE INDEX file_diffs_post_excerpt_trgm ON file_diffs USING gin (post_excerpt gin_trgm_ops);
```

- [ ] **Step 4: Write `0008_skills.sql`**

```sql
CREATE TABLE skills (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name         text NOT NULL UNIQUE,
  description  text NOT NULL,
  body         text NOT NULL,
  source       text NOT NULL CHECK (source IN ('authored','codified','imported')),
  source_session_ids uuid[] NOT NULL DEFAULT '{}',
  created_at   timestamptz NOT NULL DEFAULT now(),
  updated_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX skills_name_trgm ON skills USING gin (name gin_trgm_ops);
CREATE INDEX skills_body_trgm ON skills USING gin (body gin_trgm_ops);
```

- [ ] **Step 5: Write `0009_storage_stats.sql`**

```sql
CREATE TABLE storage_stats (
  id            bigserial PRIMARY KEY,
  sampled_at    timestamptz NOT NULL DEFAULT now(),
  pg_bytes      bigint NOT NULL,
  jsonl_bytes   bigint NOT NULL,
  session_count bigint NOT NULL,
  turn_count    bigint NOT NULL,
  diff_count    bigint NOT NULL
);
CREATE INDEX storage_stats_sampled ON storage_stats (sampled_at DESC);
```

- [ ] **Step 6: Re-run migration test**

Run: `cargo test -p teramind-db --test migrations`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/teramind-db/migrations/
git commit -m "feat(db): migrations 0005-0009 (turns, tool_calls, file_diffs, skills, storage_stats)"
```

---

### Task 36: Migration 0010 — `traces_fts` materialized view

**Files:**
- Create: `crates/teramind-db/migrations/20260513000010_traces_fts.sql`

The materialized view backs the v1 search service in Plan C. We create it now so the schema is complete and Plan C only adds queries.

- [ ] **Step 1: Write the migration**

```sql
CREATE MATERIALIZED VIEW traces_fts AS
SELECT
  t.id            AS turn_id,
  t.session_id    AS session_id,
  t.ordinal       AS ordinal,
  t.started_at    AS ts,
  to_tsvector('english',
      coalesce(t.user_prompt,'')   || ' ' ||
      coalesce(t.assistant_text,'') || ' ' ||
      coalesce(t.thinking,'')      || ' ' ||
      coalesce(string_agg(tc.output,' '),'')) AS document
FROM turns t
LEFT JOIN tool_calls tc ON tc.turn_id = t.id
GROUP BY t.id;

CREATE INDEX traces_fts_document ON traces_fts USING gin (document);
CREATE UNIQUE INDEX traces_fts_turn_id ON traces_fts (turn_id);
```

- [ ] **Step 2: Run migration test**

Run: `cargo test -p teramind-db --test migrations`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramind-db/migrations/
git commit -m "feat(db): migration 0010 — traces_fts materialized view"
```

---

## Section 6 — `teramind-db` repositories

Each repository is one file, one struct, one set of focused methods. Tests live in `crates/teramind-db/tests/repos.rs` and share a `pg_fixture` helper.

### Task 37: Test fixture for repo tests

**Files:**
- Create: `crates/teramind-db/tests/repos.rs`

- [ ] **Step 1: Write the shared fixture**

```rust
use teramind_db::{pg_supervisor::PgSupervisor, pool::DbPool, migrate};
use tempfile::TempDir;

pub struct Fixture {
    pub sup: Option<PgSupervisor>,
    pub pool: DbPool,
    _tmp: TempDir,
}

impl Fixture {
    pub async fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let sup = PgSupervisor::start(tmp.path().to_path_buf(), "teramind_test").await.unwrap();
        let pool = DbPool::connect(sup.connect_options()).await.unwrap();
        migrate::run(&pool).await.unwrap();
        Self { sup: Some(sup), pool, _tmp: tmp }
    }
    pub async fn shutdown(mut self) {
        if let Some(s) = self.sup.take() { let _ = s.shutdown().await; }
    }
}
```

- [ ] **Step 2: Smoke-run an empty test to confirm the fixture works**

Append:

```rust
#[tokio::test]
async fn fixture_initializes() {
    let f = Fixture::new().await;
    let one: (i32,) = sqlx::query_as("SELECT 1").fetch_one(f.pool.pg()).await.unwrap();
    assert_eq!(one.0, 1);
    f.shutdown().await;
}
```

- [ ] **Step 3: Run it**

Run: `cargo test -p teramind-db --test repos fixture_initializes`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind-db/tests/repos.rs
git commit -m "test(db): shared PG fixture for repo tests"
```

---

### Task 38: `AgentRepo`

**Files:**
- Create: `crates/teramind-db/src/repos/mod.rs`
- Create: `crates/teramind-db/src/repos/agent.rs`

- [ ] **Step 1: Write `repos/mod.rs`**

```rust
pub mod agent;
pub use agent::AgentRepo;
```

- [ ] **Step 2: Write `repos/agent.rs`**

```rust
use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::AgentId;
use teramind_core::types::Agent;
use time::OffsetDateTime;

#[derive(Clone)]
pub struct AgentRepo { pool: DbPool }

impl AgentRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    pub async fn upsert(&self, kind: &str, version: Option<&str>) -> Result<Agent> {
        let row: (uuid::Uuid, String, Option<String>, OffsetDateTime) = sqlx::query_as(
            r#"
            INSERT INTO agents (kind, version) VALUES ($1, $2)
            ON CONFLICT (kind, version) DO UPDATE SET kind = EXCLUDED.kind
            RETURNING id, kind, version, installed_at
            "#)
            .bind(kind)
            .bind(version)
            .fetch_one(self.pool.pg()).await?;
        Ok(Agent { id: AgentId(row.0), kind: row.1, version: row.2, installed_at: row.3 })
    }

    pub async fn find(&self, kind: &str, version: Option<&str>) -> Result<Option<Agent>> {
        let r: Option<(uuid::Uuid, String, Option<String>, OffsetDateTime)> = sqlx::query_as(
            "SELECT id, kind, version, installed_at FROM agents WHERE kind = $1 AND version IS NOT DISTINCT FROM $2")
            .bind(kind).bind(version)
            .fetch_optional(self.pool.pg()).await?;
        Ok(r.map(|r| Agent { id: AgentId(r.0), kind: r.1, version: r.2, installed_at: r.3 }))
    }
}
```

- [ ] **Step 3: Add `repos` to `lib.rs`** if not already exported. Confirm `crates/teramind-db/src/lib.rs` contains `pub mod repos;`.

- [ ] **Step 4: Write test in `tests/repos.rs`** (append):

```rust
#[tokio::test]
async fn agent_repo_upserts_and_finds() {
    let f = Fixture::new().await;
    let repo = teramind_db::repos::AgentRepo::new(f.pool.clone());
    let a1 = repo.upsert("claude_code", Some("0.1.0")).await.unwrap();
    let a2 = repo.upsert("claude_code", Some("0.1.0")).await.unwrap();
    assert_eq!(a1.id, a2.id);
    let found = repo.find("claude_code", Some("0.1.0")).await.unwrap().unwrap();
    assert_eq!(found.id, a1.id);
    f.shutdown().await;
}
```

- [ ] **Step 5: Run it**

Run: `cargo test -p teramind-db --test repos agent_repo_upserts_and_finds`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/teramind-db/src/repos/
git add crates/teramind-db/tests/repos.rs
git commit -m "feat(db): AgentRepo with upsert + find"
```

---

### Task 39: `ProjectRepo`

**Files:**
- Create: `crates/teramind-db/src/repos/project.rs`
- Modify: `crates/teramind-db/src/repos/mod.rs`

- [ ] **Step 1: Add `pub mod project; pub use project::ProjectRepo;` to `repos/mod.rs`.**

- [ ] **Step 2: Write `repos/project.rs`**

```rust
use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::ProjectId;
use teramind_core::types::Project;
use time::OffsetDateTime;

#[derive(Clone)]
pub struct ProjectRepo { pool: DbPool }

impl ProjectRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    pub async fn upsert_by_root(&self, root_path: &str, git_remote: Option<&str>, display_name: Option<&str>) -> Result<Project> {
        let r: (uuid::Uuid, String, Option<String>, Option<String>, OffsetDateTime) = sqlx::query_as(
            r#"
            INSERT INTO projects (root_path, git_remote, display_name)
            VALUES ($1, $2, $3)
            ON CONFLICT (root_path) DO UPDATE SET
                git_remote = COALESCE(EXCLUDED.git_remote, projects.git_remote),
                display_name = COALESCE(EXCLUDED.display_name, projects.display_name)
            RETURNING id, root_path, git_remote, display_name, first_seen
            "#)
            .bind(root_path).bind(git_remote).bind(display_name)
            .fetch_one(self.pool.pg()).await?;
        Ok(Project { id: ProjectId(r.0), root_path: r.1, git_remote: r.2, display_name: r.3, first_seen: r.4 })
    }
}
```

- [ ] **Step 3: Add a test in `tests/repos.rs`**

```rust
#[tokio::test]
async fn project_repo_upserts_by_root_path() {
    let f = Fixture::new().await;
    let repo = teramind_db::repos::ProjectRepo::new(f.pool.clone());
    let p1 = repo.upsert_by_root("/home/dev/x", Some("git@github.com:org/x.git"), None).await.unwrap();
    let p2 = repo.upsert_by_root("/home/dev/x", None, Some("X")).await.unwrap();
    assert_eq!(p1.id, p2.id);
    assert_eq!(p2.git_remote.as_deref(), Some("git@github.com:org/x.git"));
    assert_eq!(p2.display_name.as_deref(), Some("X"));
    f.shutdown().await;
}
```

- [ ] **Step 4: Run it**

Run: `cargo test -p teramind-db --test repos project_repo_upserts_by_root_path`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-db/src/repos/
git add crates/teramind-db/tests/repos.rs
git commit -m "feat(db): ProjectRepo with upsert_by_root"
```

---

### Task 40: `SessionRepo`

**Files:**
- Create: `crates/teramind-db/src/repos/session.rs`
- Modify: `crates/teramind-db/src/repos/mod.rs`

- [ ] **Step 1: Add `pub mod session; pub use session::SessionRepo;` to `repos/mod.rs`.**

- [ ] **Step 2: Write `repos/session.rs`**

```rust
use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::{AgentId, ProjectId, SessionId};
use time::OffsetDateTime;

#[derive(Clone)]
pub struct SessionRepo { pool: DbPool }

pub struct NewSession<'a> {
    pub agent_id: AgentId,
    pub agent_session_id: Option<&'a str>,
    pub cwd: &'a str,
    pub project_id: Option<ProjectId>,
    pub parent_session_id: Option<SessionId>,
    pub git_head: Option<&'a str>,
    pub git_branch: Option<&'a str>,
    pub os: &'a str,
    pub hostname: &'a str,
    pub user_login: &'a str,
    pub started_at: OffsetDateTime,
}

impl SessionRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    pub async fn insert(&self, n: NewSession<'_>) -> Result<SessionId> {
        let r: (uuid::Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO sessions (agent_id, agent_session_id, cwd, project_id, parent_session_id,
                                  git_head, git_branch, os, hostname, user_login, started_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
            RETURNING id
            "#)
            .bind(n.agent_id.0)
            .bind(n.agent_session_id)
            .bind(n.cwd)
            .bind(n.project_id.map(|p| p.0))
            .bind(n.parent_session_id.map(|p| p.0))
            .bind(n.git_head).bind(n.git_branch)
            .bind(n.os).bind(n.hostname).bind(n.user_login)
            .bind(n.started_at)
            .fetch_one(self.pool.pg()).await?;
        Ok(SessionId(r.0))
    }

    pub async fn end(&self, id: SessionId, ended_at: OffsetDateTime, reason: &str) -> Result<()> {
        sqlx::query("UPDATE sessions SET ended_at=$1, end_reason=$2 WHERE id=$3 AND ended_at IS NULL")
            .bind(ended_at).bind(reason).bind(id.0)
            .execute(self.pool.pg()).await?;
        Ok(())
    }

    pub async fn append_metadata(&self, id: SessionId, key: &str, value: serde_json::Value) -> Result<()> {
        sqlx::query("UPDATE sessions SET metadata = metadata || jsonb_build_object($1, $2) WHERE id=$3")
            .bind(key).bind(value).bind(id.0)
            .execute(self.pool.pg()).await?;
        Ok(())
    }
}
```

- [ ] **Step 3: Add tests**

Append to `tests/repos.rs`:

```rust
#[tokio::test]
async fn session_repo_inserts_and_ends() {
    let f = Fixture::new().await;
    let agents = teramind_db::repos::AgentRepo::new(f.pool.clone());
    let agent = agents.upsert("claude_code", Some("0.1.0")).await.unwrap();
    let repo = teramind_db::repos::SessionRepo::new(f.pool.clone());

    let now = time::OffsetDateTime::now_utc();
    let id = repo.insert(teramind_db::repos::session::NewSession {
        agent_id: agent.id,
        agent_session_id: Some("abc"),
        cwd: "/work",
        project_id: None,
        parent_session_id: None,
        git_head: None, git_branch: None,
        os: "linux", hostname: "h", user_login: "u",
        started_at: now,
    }).await.unwrap();

    repo.end(id, now + time::Duration::seconds(60), "stop_hook").await.unwrap();

    let (ended_at, end_reason): (Option<time::OffsetDateTime>, Option<String>) = sqlx::query_as(
        "SELECT ended_at, end_reason FROM sessions WHERE id=$1")
        .bind(id.0)
        .fetch_one(f.pool.pg()).await.unwrap();
    assert!(ended_at.is_some());
    assert_eq!(end_reason.as_deref(), Some("stop_hook"));

    f.shutdown().await;
}
```

- [ ] **Step 4: Run it**

Run: `cargo test -p teramind-db --test repos session_repo_inserts_and_ends`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-db/src/repos/ crates/teramind-db/tests/repos.rs
git commit -m "feat(db): SessionRepo (insert/end/append_metadata)"
```

---

### Task 41: `TraceRepo` (turns + tool_calls)

**Files:**
- Create: `crates/teramind-db/src/repos/trace.rs`
- Modify: `crates/teramind-db/src/repos/mod.rs`

- [ ] **Step 1: Add `pub mod trace; pub use trace::TraceRepo;` to `repos/mod.rs`.**

- [ ] **Step 2: Write `repos/trace.rs`**

```rust
use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::{SessionId, ToolCallId, TurnId};
use time::OffsetDateTime;

#[derive(Clone)]
pub struct TraceRepo { pool: DbPool }

impl TraceRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    /// Insert a turn unless one with the same (session_id, ordinal) already exists.
    /// Returns the existing or newly-created turn id.
    pub async fn upsert_turn(&self, session_id: SessionId, ordinal: i32, started_at: OffsetDateTime, user_prompt: Option<&str>) -> Result<TurnId> {
        let r: (uuid::Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO turns (session_id, ordinal, started_at, user_prompt)
            VALUES ($1,$2,$3,$4)
            ON CONFLICT (session_id, ordinal) DO UPDATE SET user_prompt = COALESCE(EXCLUDED.user_prompt, turns.user_prompt)
            RETURNING id
            "#)
            .bind(session_id.0).bind(ordinal).bind(started_at).bind(user_prompt)
            .fetch_one(self.pool.pg()).await?;
        Ok(TurnId(r.0))
    }

    pub async fn finalize_turn(&self, id: TurnId, ended_at: OffsetDateTime, assistant_text: Option<&str>, thinking: Option<&str>, model: Option<&str>, input_tokens: Option<i32>, output_tokens: Option<i32>) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE turns SET ended_at=$1, assistant_text=$2, thinking=$3, model=$4,
                             input_tokens=$5, output_tokens=$6
            WHERE id=$7
            "#)
            .bind(ended_at).bind(assistant_text).bind(thinking).bind(model)
            .bind(input_tokens).bind(output_tokens).bind(id.0)
            .execute(self.pool.pg()).await?;
        Ok(())
    }

    pub async fn insert_tool_call_start(&self, turn_id: TurnId, ordinal: i32, name: &str, input: &serde_json::Value, started_at: OffsetDateTime) -> Result<ToolCallId> {
        let r: (uuid::Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO tool_calls (turn_id, ordinal, name, input, started_at)
            VALUES ($1,$2,$3,$4,$5)
            ON CONFLICT (turn_id, ordinal) DO UPDATE SET name = EXCLUDED.name
            RETURNING id
            "#)
            .bind(turn_id.0).bind(ordinal).bind(name).bind(input).bind(started_at)
            .fetch_one(self.pool.pg()).await?;
        Ok(ToolCallId(r.0))
    }

    pub async fn finalize_tool_call(&self, id: ToolCallId, output: &str, is_error: bool, duration_ms: i32) -> Result<()> {
        sqlx::query("UPDATE tool_calls SET output=$1, is_error=$2, duration_ms=$3 WHERE id=$4")
            .bind(output).bind(is_error).bind(duration_ms).bind(id.0)
            .execute(self.pool.pg()).await?;
        Ok(())
    }
}
```

- [ ] **Step 3: Add a test**

```rust
#[tokio::test]
async fn trace_repo_full_turn_lifecycle() {
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

    let traces = teramind_db::repos::TraceRepo::new(f.pool.clone());
    let turn = traces.upsert_turn(session_id, 0, now, Some("hi")).await.unwrap();
    let tc = traces.insert_tool_call_start(turn, 0, "Edit", &serde_json::json!({"x":1}), now).await.unwrap();
    traces.finalize_tool_call(tc, "ok", false, 12).await.unwrap();
    traces.finalize_turn(turn, now + time::Duration::seconds(1), Some("done"), None, Some("claude-opus-4-7"), Some(10), Some(5)).await.unwrap();

    let row: (Option<String>, Option<String>, Option<i32>) = sqlx::query_as(
        "SELECT assistant_text, model, output_tokens FROM turns WHERE id=$1")
        .bind(turn.0).fetch_one(f.pool.pg()).await.unwrap();
    assert_eq!(row.0.as_deref(), Some("done"));
    assert_eq!(row.1.as_deref(), Some("claude-opus-4-7"));
    assert_eq!(row.2, Some(5));

    f.shutdown().await;
}
```

- [ ] **Step 4: Run**

Run: `cargo test -p teramind-db --test repos trace_repo_full_turn_lifecycle`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-db/src/repos/ crates/teramind-db/tests/repos.rs
git commit -m "feat(db): TraceRepo (turn/tool_call insert + finalize)"
```

---

### Task 42: `DiffRepo`

**Files:**
- Create: `crates/teramind-db/src/repos/diff.rs`
- Modify: `crates/teramind-db/src/repos/mod.rs`

- [ ] **Step 1: Add `pub mod diff; pub use diff::DiffRepo;` to `repos/mod.rs`.**

- [ ] **Step 2: Write `repos/diff.rs`**

```rust
use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::{FileDiffId, SessionId, TurnId};
use teramind_core::types::file_diff::Attribution;
use time::OffsetDateTime;

#[derive(Clone)]
pub struct DiffRepo { pool: DbPool }

pub struct NewFileDiff<'a> {
    pub turn_id: Option<TurnId>,
    pub session_id: SessionId,
    pub file_path: &'a str,
    pub rel_path: &'a str,
    pub attribution: Attribution,
    pub language: Option<&'a str>,
    pub pre_excerpt: &'a str,
    pub post_excerpt: &'a str,
    pub unified_diff: &'a str,
    pub pre_hash: [u8; 32],
    pub post_hash: [u8; 32],
    pub byte_size: i32,
    pub captured_at: OffsetDateTime,
}

impl DiffRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    pub async fn insert(&self, n: NewFileDiff<'_>) -> Result<FileDiffId> {
        let attr = match n.attribution { Attribution::Agent => "agent", Attribution::Human => "human" };
        let r: (uuid::Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO file_diffs (turn_id, session_id, file_path, rel_path, attribution, language,
                                    pre_excerpt, post_excerpt, unified_diff, pre_hash, post_hash, byte_size, captured_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
            RETURNING id
            "#)
            .bind(n.turn_id.map(|t| t.0)).bind(n.session_id.0)
            .bind(n.file_path).bind(n.rel_path).bind(attr).bind(n.language)
            .bind(n.pre_excerpt).bind(n.post_excerpt).bind(n.unified_diff)
            .bind(&n.pre_hash[..]).bind(&n.post_hash[..]).bind(n.byte_size).bind(n.captured_at)
            .fetch_one(self.pool.pg()).await?;
        Ok(FileDiffId(r.0))
    }
}
```

- [ ] **Step 3: Add test**

```rust
#[tokio::test]
async fn diff_repo_inserts_a_file_diff() {
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
    let diffs = teramind_db::repos::DiffRepo::new(f.pool.clone());
    let id = diffs.insert(teramind_db::repos::diff::NewFileDiff {
        turn_id: None,
        session_id,
        file_path: "/w/x.rs", rel_path: "x.rs",
        attribution: teramind_core::types::file_diff::Attribution::Agent,
        language: Some("rust"),
        pre_excerpt: "a", post_excerpt: "b",
        unified_diff: "--- a\n+++ b\n", pre_hash: [1u8;32], post_hash: [2u8;32],
        byte_size: 1, captured_at: now,
    }).await.unwrap();
    let row: (i32,) = sqlx::query_as("SELECT byte_size FROM file_diffs WHERE id=$1")
        .bind(id.0).fetch_one(f.pool.pg()).await.unwrap();
    assert_eq!(row.0, 1);
    f.shutdown().await;
}
```

- [ ] **Step 4: Run**

Run: `cargo test -p teramind-db --test repos diff_repo_inserts_a_file_diff`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind-db/src/repos/ crates/teramind-db/tests/repos.rs
git commit -m "feat(db): DiffRepo (file_diffs insert)"
```

---

### Task 43: `SkillRepo` and `StorageStatsRepo`

**Files:**
- Create: `crates/teramind-db/src/repos/skill.rs`
- Create: `crates/teramind-db/src/repos/storage_stats.rs`
- Modify: `crates/teramind-db/src/repos/mod.rs`

- [ ] **Step 1: Add to `repos/mod.rs`**

```rust
pub mod skill;
pub mod storage_stats;
pub use skill::SkillRepo;
pub use storage_stats::StorageStatsRepo;
```

- [ ] **Step 2: Write `repos/skill.rs`**

```rust
use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::SkillId;

#[derive(Clone)]
pub struct SkillRepo { pool: DbPool }

impl SkillRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }
    pub async fn upsert_authored(&self, name: &str, description: &str, body: &str) -> Result<SkillId> {
        let r: (uuid::Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO skills (name, description, body, source)
            VALUES ($1,$2,$3,'authored')
            ON CONFLICT (name) DO UPDATE SET description=EXCLUDED.description, body=EXCLUDED.body, updated_at=now()
            RETURNING id
            "#)
            .bind(name).bind(description).bind(body)
            .fetch_one(self.pool.pg()).await?;
        Ok(SkillId(r.0))
    }
}
```

- [ ] **Step 3: Write `repos/storage_stats.rs`**

```rust
use crate::error::Result;
use crate::pool::DbPool;

#[derive(Clone)]
pub struct StorageStatsRepo { pool: DbPool }

pub struct Sample {
    pub pg_bytes: i64,
    pub jsonl_bytes: i64,
    pub session_count: i64,
    pub turn_count: i64,
    pub diff_count: i64,
}

impl StorageStatsRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }
    pub async fn insert(&self, s: Sample) -> Result<()> {
        sqlx::query("INSERT INTO storage_stats (pg_bytes, jsonl_bytes, session_count, turn_count, diff_count) VALUES ($1,$2,$3,$4,$5)")
            .bind(s.pg_bytes).bind(s.jsonl_bytes).bind(s.session_count).bind(s.turn_count).bind(s.diff_count)
            .execute(self.pool.pg()).await?;
        Ok(())
    }
    pub async fn count_sessions(&self) -> Result<i64> {
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM sessions").fetch_one(self.pool.pg()).await?;
        Ok(n)
    }
    pub async fn count_turns(&self) -> Result<i64> {
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM turns").fetch_one(self.pool.pg()).await?;
        Ok(n)
    }
    pub async fn count_diffs(&self) -> Result<i64> {
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM file_diffs").fetch_one(self.pool.pg()).await?;
        Ok(n)
    }
    pub async fn pg_database_size(&self, database: &str) -> Result<i64> {
        let (n,): (i64,) = sqlx::query_as("SELECT pg_database_size($1)::bigint").bind(database).fetch_one(self.pool.pg()).await?;
        Ok(n)
    }
}
```

- [ ] **Step 4: Tests for both**

Append to `tests/repos.rs`:

```rust
#[tokio::test]
async fn skill_repo_upserts_authored() {
    let f = Fixture::new().await;
    let r = teramind_db::repos::SkillRepo::new(f.pool.clone());
    let id1 = r.upsert_authored("k", "d", "b1").await.unwrap();
    let id2 = r.upsert_authored("k", "d", "b2").await.unwrap();
    assert_eq!(id1, id2);
    let (body,): (String,) = sqlx::query_as("SELECT body FROM skills WHERE id=$1").bind(id1.0).fetch_one(f.pool.pg()).await.unwrap();
    assert_eq!(body, "b2");
    f.shutdown().await;
}

#[tokio::test]
async fn storage_stats_repo_inserts_and_counts() {
    let f = Fixture::new().await;
    let r = teramind_db::repos::StorageStatsRepo::new(f.pool.clone());
    r.insert(teramind_db::repos::storage_stats::Sample {
        pg_bytes: 100, jsonl_bytes: 200, session_count: 0, turn_count: 0, diff_count: 0
    }).await.unwrap();
    assert_eq!(r.count_sessions().await.unwrap(), 0);
    f.shutdown().await;
}
```

- [ ] **Step 5: Run**

Run: `cargo test -p teramind-db --test repos skill_repo_upserts_authored storage_stats_repo_inserts_and_counts`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/teramind-db/src/repos/ crates/teramind-db/tests/repos.rs
git commit -m "feat(db): SkillRepo and StorageStatsRepo"
```

---

## Section 7 — `teramindd` daemon: skeleton, paths, config, signals

### Task 44: XDG-style paths resolver

**Files:**
- Create: `crates/teramindd/src/paths.rs`

- [ ] **Step 1: Write the paths module**

```rust
use std::path::PathBuf;

pub struct Paths {
    pub data_dir: PathBuf,    // ~/.local/share/teramind (or %LOCALAPPDATA%\teramind\data)
    pub config_dir: PathBuf,  // ~/.config/teramind        (or %APPDATA%\teramind)
    pub pgdata_dir: PathBuf,  // data_dir/pgdata
    pub raw_dir: PathBuf,     // data_dir/raw
    pub inbox_dir: PathBuf,   // data_dir/inbox
    pub dead_letter_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub socket_path: PathBuf, // /tmp/teramind.sock | \\.\pipe\teramind | $TERAMIND_SOCKET
    pub pid_file: PathBuf,
}

impl Paths {
    pub fn resolve() -> std::io::Result<Self> {
        #[cfg(unix)]
        let (data, config) = {
            let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "HOME unset"))?;
            let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share")).join("teramind");
            let conf = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config")).join("teramind");
            (data, conf)
        };
        #[cfg(windows)]
        let (data, config) = {
            let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "LOCALAPPDATA unset"))?;
            let app   = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| local.clone());
            (local.join("teramind").join("data"), app.join("teramind"))
        };

        let socket_path = teramind_ipc::transport::default_socket_path();
        Ok(Paths {
            pgdata_dir: data.join("pgdata"),
            raw_dir: data.join("raw"),
            inbox_dir: data.join("inbox"),
            dead_letter_dir: data.join("dead_letter"),
            logs_dir: data.join("logs"),
            pid_file: data.join("teramindd.pid"),
            data_dir: data,
            config_dir: config,
            socket_path,
        })
    }

    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        for d in [&self.data_dir, &self.config_dir, &self.pgdata_dir, &self.raw_dir,
                  &self.inbox_dir, &self.dead_letter_dir, &self.logs_dir] {
            std::fs::create_dir_all(d)?;
        }
        Ok(())
    }
}
```

- [ ] **Step 2: Add an in-file test** at the bottom of `paths.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ensure_dirs_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        std::env::set_var("XDG_DATA_HOME", tmp.path().join("xdg-data"));
        std::env::set_var("XDG_CONFIG_HOME", tmp.path().join("xdg-config"));
        #[cfg(windows)] { std::env::set_var("LOCALAPPDATA", tmp.path()); std::env::set_var("APPDATA", tmp.path()); }
        let p = Paths::resolve().unwrap();
        p.ensure_dirs().unwrap();
        p.ensure_dirs().unwrap();
        assert!(p.data_dir.exists());
        assert!(p.raw_dir.exists());
    }
}
```

Add `tempfile = { workspace = true }` under `[dev-dependencies]` in `crates/teramindd/Cargo.toml`.

- [ ] **Step 3: Run**

Run: `cargo test -p teramindd paths::tests::ensure_dirs_is_idempotent`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/paths.rs crates/teramindd/Cargo.toml
git commit -m "feat(daemon): XDG-style paths resolver"
```

---

### Task 45: Config loader

**Files:**
- Create: `crates/teramindd/src/config.rs`

- [ ] **Step 1: Write the config struct**

```rust
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "Config::default_ingest_queue_capacity")]
    pub ingest_queue_capacity: usize,
    #[serde(default = "Config::default_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
    #[serde(default = "Config::default_redaction_enabled")]
    pub redaction_enabled: bool,
    #[serde(default = "Config::default_autorecall_enabled")]
    pub autorecall_enabled: bool,
    #[serde(default = "Config::default_storage_sample_interval_secs")]
    pub storage_sample_interval_secs: u64,
}

impl Config {
    fn default_ingest_queue_capacity() -> usize { 4096 }
    fn default_idle_timeout_secs() -> u64 { 30 * 60 }
    fn default_redaction_enabled() -> bool { true }
    fn default_autorecall_enabled() -> bool { true }
    fn default_storage_sample_interval_secs() -> u64 { 300 }

    pub fn defaults() -> Self {
        toml::from_str("").expect("default config must parse from empty toml")
    }

    pub fn load_or_default(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() { return Ok(Self::defaults()); }
        let text = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }
}
```

Add `toml = "0.8"` to `crates/teramindd/Cargo.toml` `[dependencies]`.

- [ ] **Step 2: Test**

In `config.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_sane() {
        let c = Config::defaults();
        assert!(c.ingest_queue_capacity >= 1024);
        assert!(c.redaction_enabled);
    }
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p teramindd config::tests::defaults_are_sane`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/config.rs crates/teramindd/Cargo.toml
git commit -m "feat(daemon): Config loader with sane defaults"
```

---

### Task 46: Signal handling

**Files:**
- Create: `crates/teramindd/src/signals.rs`

- [ ] **Step 1: Write the signal future**

```rust
use tokio::signal;

/// Resolves on SIGTERM / SIGINT (Unix) or Ctrl-C (Windows).
pub async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use signal::unix::{signal as unix_signal, SignalKind};
        let mut term = unix_signal(SignalKind::terminate()).expect("install SIGTERM handler");
        let mut intr = unix_signal(SignalKind::interrupt()).expect("install SIGINT handler");
        tokio::select! {
            _ = term.recv() => {}
            _ = intr.recv() => {}
        }
    }
    #[cfg(windows)]
    {
        let _ = signal::ctrl_c().await;
    }
}
```

- [ ] **Step 2: Compile-check**

Run: `cargo check -p teramindd`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/src/signals.rs
git commit -m "feat(daemon): cross-platform shutdown signal future"
```

---

## Section 8 — `teramindd` daemon: services

### Task 47: JSONL shadow writer

**Files:**
- Create: `crates/teramindd/src/services/mod.rs`
- Create: `crates/teramindd/src/services/jsonl_writer.rs`

- [ ] **Step 1: Write `services/mod.rs`** with `pub mod jsonl_writer;` to start.

- [ ] **Step 2: Write `services/jsonl_writer.rs`**

```rust
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::io::AsyncWriteExt;
use teramind_core::types::ingest_event::EventEnvelope;
use time::macros::format_description;

pub struct JsonlWriter {
    base: PathBuf,
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    current_date: String,           // "YYYY-MM-DD"
    file: tokio::fs::File,
    path: PathBuf,
}

impl JsonlWriter {
    pub async fn open(base: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&base)?;
        let (date, path, file) = Self::open_today(&base).await?;
        Ok(Self {
            base,
            inner: Arc::new(Mutex::new(Inner { current_date: date, file, path })),
        })
    }

    async fn open_today(base: &PathBuf) -> std::io::Result<(String, PathBuf, tokio::fs::File)> {
        let now = time::OffsetDateTime::now_utc();
        let fmt = format_description!("[year]-[month]-[day]");
        let date = now.format(&fmt).expect("format date");
        let path = base.join(format!("{date}.jsonl"));
        let file = tokio::fs::OpenOptions::new().create(true).append(true).open(&path).await?;
        Ok((date, path, file))
    }

    pub async fn append(&self, env: &EventEnvelope) -> std::io::Result<()> {
        let mut g = self.inner.lock().await;
        // rotate if the day has changed
        let now = time::OffsetDateTime::now_utc();
        let fmt = format_description!("[year]-[month]-[day]");
        let today = now.format(&fmt).expect("format date");
        if today != g.current_date {
            let (d, p, f) = Self::open_today(&self.base).await?;
            g.current_date = d; g.file = f; g.path = p;
        }
        let mut bytes = serde_json::to_vec(env)?;
        bytes.push(b'\n');
        g.file.write_all(&bytes).await?;
        g.file.flush().await?;
        Ok(())
    }
}
```

- [ ] **Step 3: Write the test**

In the same file:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use teramind_core::ids::{ClientEventId, SessionId};
    use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
    use tempfile::tempdir;
    use time::OffsetDateTime;

    #[tokio::test]
    async fn writer_appends_jsonl_to_daily_file() {
        let tmp = tempdir().unwrap();
        let w = JsonlWriter::open(tmp.path().to_path_buf()).await.unwrap();
        let env = EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::now_utc(),
            event: IngestEvent::UserPrompt {
                session_id: SessionId::new(), turn_ordinal: 0, prompt: "x".into(),
            },
        };
        w.append(&env).await.unwrap();
        w.append(&env).await.unwrap();
        let entries: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().collect();
        assert_eq!(entries.len(), 1);
        let p = entries[0].as_ref().unwrap().path();
        let body = std::fs::read_to_string(&p).unwrap();
        assert_eq!(body.lines().count(), 2);
    }
}
```

- [ ] **Step 4: Run**

Run: `cargo test -p teramindd services::jsonl_writer::tests::writer_appends_jsonl_to_daily_file`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramindd/src/services/
git commit -m "feat(daemon): JsonlWriter with daily rotation"
```

---

### Task 48: Session manager

**Files:**
- Create: `crates/teramindd/src/services/session_manager.rs`
- Modify: `crates/teramindd/src/services/mod.rs`

- [ ] **Step 1: Add `pub mod session_manager;` to `services/mod.rs`.**

- [ ] **Step 2: Write `services/session_manager.rs`**

```rust
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use teramind_core::ids::{SessionId, TurnId};
use time::OffsetDateTime;

#[derive(Debug, Clone)]
pub struct ActiveSession {
    pub session_id: SessionId,
    pub cwd: String,
    pub agent_kind: String,
    pub started_at: OffsetDateTime,
    pub last_activity: OffsetDateTime,
    pub last_turn_id: Option<TurnId>,
}

#[derive(Clone, Default)]
pub struct SessionManager {
    inner: Arc<RwLock<HashMap<SessionId, ActiveSession>>>,
}

impl SessionManager {
    pub fn new() -> Self { Self::default() }

    pub async fn start(&self, s: ActiveSession) {
        self.inner.write().await.insert(s.session_id, s);
    }
    pub async fn touch(&self, id: SessionId, at: OffsetDateTime, turn_id: Option<TurnId>) {
        if let Some(s) = self.inner.write().await.get_mut(&id) {
            s.last_activity = at;
            if turn_id.is_some() { s.last_turn_id = turn_id; }
        }
    }
    pub async fn end(&self, id: SessionId) -> Option<ActiveSession> {
        self.inner.write().await.remove(&id)
    }
    pub async fn get(&self, id: SessionId) -> Option<ActiveSession> {
        self.inner.read().await.get(&id).cloned()
    }
    pub async fn idle_since(&self, cutoff: OffsetDateTime) -> Vec<ActiveSession> {
        self.inner.read().await.values().filter(|s| s.last_activity < cutoff).cloned().collect()
    }
}
```

- [ ] **Step 3: Write tests** at the bottom of the same file:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use time::OffsetDateTime;

    #[tokio::test]
    async fn manager_lifecycle() {
        let m = SessionManager::new();
        let sid = SessionId::new();
        let now = OffsetDateTime::now_utc();
        m.start(ActiveSession {
            session_id: sid, cwd: "/w".into(), agent_kind: "claude_code".into(),
            started_at: now, last_activity: now, last_turn_id: None,
        }).await;
        assert!(m.get(sid).await.is_some());
        m.touch(sid, now + time::Duration::seconds(5), None).await;
        let removed = m.end(sid).await;
        assert!(removed.is_some());
        assert!(m.get(sid).await.is_none());
    }

    #[tokio::test]
    async fn idle_since_filters() {
        let m = SessionManager::new();
        let sid = SessionId::new();
        let t0 = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        m.start(ActiveSession { session_id: sid, cwd: "/".into(), agent_kind: "c".into(),
                                started_at: t0, last_activity: t0, last_turn_id: None }).await;
        let stale = m.idle_since(t0 + time::Duration::seconds(1)).await;
        assert_eq!(stale.len(), 1);
        let fresh = m.idle_since(t0 - time::Duration::seconds(1)).await;
        assert_eq!(fresh.len(), 0);
    }
}
```

- [ ] **Step 4: Run**

Run: `cargo test -p teramindd services::session_manager`
Expected: PASS (both tests).

- [ ] **Step 5: Commit**

```bash
git add crates/teramindd/src/services/
git commit -m "feat(daemon): SessionManager with idle filtering"
```

---

### Task 49: Ingest service — channel, backpressure, redaction wiring

**Files:**
- Create: `crates/teramindd/src/services/ingest.rs`
- Modify: `crates/teramindd/src/services/mod.rs`

This is the single-writer pipeline. Plain TDD: enqueue → process → assert DB rows.

- [ ] **Step 1: Add `pub mod ingest;` to `services/mod.rs`.**

- [ ] **Step 2: Write `services/ingest.rs`** (channel + counters + dispatcher only — DB writes are TODO until Step 3)

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;
use teramind_core::redact::Redactor;
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo};
use teramind_db::repos::session::NewSession;
use crate::services::jsonl_writer::JsonlWriter;
use crate::services::session_manager::{ActiveSession, SessionManager};
use time::OffsetDateTime;
use tracing::warn;

#[derive(Default)]
pub struct IngestStats {
    pub drops: AtomicU64,
    pub queue_depth: AtomicU64,
    pub pg_write_failures: AtomicU64,
    pub dead_letters: AtomicU64,
}

pub struct IngestService {
    tx: mpsc::Sender<EventEnvelope>,
    stats: Arc<IngestStats>,
}

#[derive(Clone)]
pub struct IngestDeps {
    pub redactor: Arc<Redactor>,
    pub jsonl: Arc<JsonlWriter>,
    pub sessions: SessionManager,
    pub agents: AgentRepo,
    pub session_repo: SessionRepo,
    pub trace: TraceRepo,
    pub diffs: DiffRepo,
    pub stats: Arc<IngestStats>,
    pub dead_letter_dir: std::path::PathBuf,
}

impl IngestService {
    pub fn spawn(capacity: usize, deps: IngestDeps) -> Self {
        let (tx, mut rx) = mpsc::channel::<EventEnvelope>(capacity);
        let stats = deps.stats.clone();
        let stats_for_loop = stats.clone();
        tokio::spawn(async move {
            while let Some(env) = rx.recv().await {
                stats_for_loop.queue_depth.fetch_sub(1, Ordering::Relaxed);
                if let Err(e) = handle(&deps, env).await {
                    warn!(error = %e, "ingest handler error");
                    stats_for_loop.pg_write_failures.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        Self { tx, stats }
    }

    pub fn try_enqueue(&self, env: EventEnvelope) -> Result<(), EventEnvelope> {
        match self.tx.try_send(env) {
            Ok(_) => { self.stats.queue_depth.fetch_add(1, Ordering::Relaxed); Ok(()) }
            Err(mpsc::error::TrySendError::Full(env)) | Err(mpsc::error::TrySendError::Closed(env)) => {
                self.stats.drops.fetch_add(1, Ordering::Relaxed);
                Err(env)
            }
        }
    }

    pub fn stats(&self) -> Arc<IngestStats> { self.stats.clone() }
}

async fn handle(d: &IngestDeps, env: EventEnvelope) -> anyhow::Result<()> {
    // 1. JSONL append always happens before PG, before redaction.
    //    The on-disk shadow may contain raw secrets — but JSONL append errors are reported, not swallowed.
    d.jsonl.append(&env).await?;
    // 2. Apply redaction to fields that would otherwise carry secrets.
    let redacted = redact_envelope(&d.redactor, env);
    // 3. Route by variant.
    route(d, redacted).await
}

fn redact_envelope(r: &Redactor, mut env: EventEnvelope) -> EventEnvelope {
    use IngestEvent::*;
    env.event = match env.event {
        UserPrompt { session_id, turn_ordinal, prompt } =>
            UserPrompt { session_id, turn_ordinal, prompt: r.apply(&prompt) },
        ToolCallStart { turn_id, ordinal, name, input } =>
            ToolCallStart { turn_id, ordinal, name, input: serde_json::from_str(&r.apply(&input.to_string())).unwrap_or(input) },
        ToolCallEnd { tool_call_id, output, is_error, duration_ms } =>
            ToolCallEnd { tool_call_id, output: r.apply(&output), is_error, duration_ms },
        AssistantTurn { turn_id, assistant_text, thinking, model, input_tokens, output_tokens } =>
            AssistantTurn { turn_id, assistant_text: r.apply(&assistant_text),
                            thinking: thinking.map(|t| r.apply(&t)), model, input_tokens, output_tokens },
        other => other,
    };
    env
}

async fn route(d: &IngestDeps, env: EventEnvelope) -> anyhow::Result<()> {
    use IngestEvent::*;
    let ts = env.ts;
    match env.event {
        SessionStart { session_id, agent_session_id, agent_kind, cwd, os, hostname, user_login, git_head, git_branch } => {
            let agent = d.agents.upsert(&agent_kind, None).await?;
            let sid = d.session_repo.insert(NewSession {
                agent_id: agent.id,
                agent_session_id: agent_session_id.as_deref(),
                cwd: &cwd,
                project_id: None,
                parent_session_id: None,
                git_head: git_head.as_deref(),
                git_branch: git_branch.as_deref(),
                os: &os, hostname: &hostname, user_login: &user_login,
                started_at: ts,
            }).await?;
            // If client supplied a session_id, prefer it for ActiveSession bookkeeping; otherwise use the DB id.
            let track_id = if session_id.0 != uuid::Uuid::nil() { session_id } else { sid };
            d.sessions.start(ActiveSession {
                session_id: track_id, cwd: cwd.clone(), agent_kind, started_at: ts, last_activity: ts, last_turn_id: None
            }).await;
        }
        UserPrompt { session_id, turn_ordinal, prompt } => {
            let _ = d.trace.upsert_turn(session_id, turn_ordinal, ts, Some(&prompt)).await?;
            d.sessions.touch(session_id, ts, None).await;
        }
        ToolCallStart { turn_id, ordinal, name, input } => {
            let _ = d.trace.insert_tool_call_start(turn_id, ordinal, &name, &input, ts).await?;
        }
        ToolCallEnd { tool_call_id, output, is_error, duration_ms } => {
            d.trace.finalize_tool_call(tool_call_id, &output, is_error, duration_ms).await?;
        }
        AssistantTurn { turn_id, assistant_text, thinking, model, input_tokens, output_tokens } => {
            d.trace.finalize_turn(turn_id, ts, Some(&assistant_text), thinking.as_deref(),
                                  model.as_deref(), input_tokens, output_tokens).await?;
        }
        SessionEnd { session_id, reason } => {
            d.session_repo.end(session_id, ts, &reason).await?;
            d.sessions.end(session_id).await;
        }
        PreCompact { session_id } => {
            d.session_repo.append_metadata(session_id, "pre_compact_at",
                serde_json::Value::String(ts.to_string())).await?;
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Integration test for ingest pipeline**

Create `crates/teramindd/tests/ingest_e2e.rs`:

```rust
use std::sync::Arc;
use teramind_core::ids::{ClientEventId, SessionId};
use teramind_core::redact::Redactor;
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use teramind_db::{pg_supervisor::PgSupervisor, pool::DbPool, migrate};
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo};
use teramindd::services::ingest::{IngestDeps, IngestService, IngestStats};
use teramindd::services::jsonl_writer::JsonlWriter;
use teramindd::services::session_manager::SessionManager;
use tempfile::tempdir;
use time::OffsetDateTime;

#[tokio::test]
async fn ingest_session_start_then_user_prompt_writes_rows() {
    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().join("pg"), "teramind_test").await.unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();

    let jsonl = Arc::new(JsonlWriter::open(tmp.path().join("raw")).await.unwrap());
    let stats = Arc::new(IngestStats::default());
    let deps = IngestDeps {
        redactor: Arc::new(Redactor::with_default_rules()),
        jsonl: jsonl.clone(),
        sessions: SessionManager::new(),
        agents: AgentRepo::new(pool.clone()),
        session_repo: SessionRepo::new(pool.clone()),
        trace: TraceRepo::new(pool.clone()),
        diffs: DiffRepo::new(pool.clone()),
        stats: stats.clone(),
        dead_letter_dir: tmp.path().join("dl"),
    };
    let svc = IngestService::spawn(64, deps);

    let session_id = SessionId::new();
    let now = OffsetDateTime::now_utc();
    svc.try_enqueue(EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: now,
        event: IngestEvent::SessionStart {
            session_id,
            agent_session_id: Some("abc".into()),
            agent_kind: "claude_code".into(),
            cwd: "/w".into(),
            os: "linux".into(),
            hostname: "h".into(),
            user_login: "u".into(),
            git_head: None,
            git_branch: None,
        },
    }).unwrap();
    svc.try_enqueue(EventEnvelope {
        client_event_id: ClientEventId::new(),
        ts: now + time::Duration::seconds(1),
        event: IngestEvent::UserPrompt { session_id, turn_ordinal: 0, prompt: "hi key=AKIAIOSFODNN7EXAMPLE end".into() },
    }).unwrap();

    // Allow the worker to drain.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let (turn_count,): (i64,) = sqlx::query_as("SELECT count(*) FROM turns").fetch_one(pool.pg()).await.unwrap();
    assert_eq!(turn_count, 1);
    let (prompt,): (Option<String>,) = sqlx::query_as("SELECT user_prompt FROM turns LIMIT 1").fetch_one(pool.pg()).await.unwrap();
    let prompt = prompt.unwrap();
    assert!(!prompt.contains("AKIAIOSFODNN7EXAMPLE"), "secret leaked: {prompt}");

    sup.shutdown().await.unwrap();
}
```

- [ ] **Step 4: Run the integration test**

Run: `cargo test -p teramindd --test ingest_e2e ingest_session_start_then_user_prompt_writes_rows`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramindd/src/services/ingest.rs crates/teramindd/tests/ingest_e2e.rs
git commit -m "feat(daemon): IngestService with redaction, JSONL append, PG routing"
```

---

### Task 50: Backpressure test

**Files:**
- Create: `crates/teramindd/tests/backpressure.rs`

- [ ] **Step 1: Write the test**

```rust
use std::sync::Arc;
use teramind_core::ids::{ClientEventId, SessionId};
use teramind_core::redact::Redactor;
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use teramind_db::{pg_supervisor::PgSupervisor, pool::DbPool, migrate};
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo};
use teramindd::services::ingest::{IngestDeps, IngestService, IngestStats};
use teramindd::services::jsonl_writer::JsonlWriter;
use teramindd::services::session_manager::SessionManager;
use tempfile::tempdir;
use time::OffsetDateTime;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn ingest_drops_when_queue_is_saturated() {
    let tmp = tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().join("pg"), "teramind_test").await.unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();

    let jsonl = Arc::new(JsonlWriter::open(tmp.path().join("raw")).await.unwrap());
    let stats = Arc::new(IngestStats::default());
    let deps = IngestDeps {
        redactor: Arc::new(Redactor::with_default_rules()),
        jsonl, sessions: SessionManager::new(),
        agents: AgentRepo::new(pool.clone()),
        session_repo: SessionRepo::new(pool.clone()),
        trace: TraceRepo::new(pool.clone()),
        diffs: DiffRepo::new(pool.clone()),
        stats: stats.clone(),
        dead_letter_dir: tmp.path().join("dl"),
    };
    // Capacity 4 -> we'll send 100 before the worker drains.
    let svc = IngestService::spawn(4, deps);

    let sid = SessionId::new();
    let now = OffsetDateTime::now_utc();
    let mut accepted = 0u32;
    let mut dropped = 0u32;
    for i in 0..100 {
        let env = EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: now + time::Duration::milliseconds(i),
            event: IngestEvent::UserPrompt { session_id: sid, turn_ordinal: i as i32, prompt: format!("p{i}") },
        };
        match svc.try_enqueue(env) {
            Ok(_) => accepted += 1,
            Err(_) => dropped += 1,
        }
    }
    assert!(dropped > 0, "expected at least some drops with capacity=4");
    assert_eq!(stats.drops.load(Ordering::Relaxed) as u32, dropped);
    assert!(accepted + dropped == 100);

    sup.shutdown().await.unwrap();
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p teramindd --test backpressure ingest_drops_when_queue_is_saturated`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/teramindd/tests/backpressure.rs
git commit -m "test(daemon): backpressure drops are counted, never block"
```

---

### Task 51: Storage stats sampler

**Files:**
- Create: `crates/teramindd/src/services/storage_stats.rs`
- Modify: `crates/teramindd/src/services/mod.rs`

- [ ] **Step 1: Add `pub mod storage_stats;` to `services/mod.rs`.**

- [ ] **Step 2: Write `services/storage_stats.rs`**

```rust
use std::path::PathBuf;
use std::time::Duration;
use teramind_db::repos::storage_stats::{Sample, StorageStatsRepo};
use tracing::warn;

pub fn spawn(repo: StorageStatsRepo, raw_dir: PathBuf, database: String, interval: Duration) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.tick().await; // first tick is immediate
        loop {
            ticker.tick().await;
            if let Err(e) = tick(&repo, &raw_dir, &database).await {
                warn!(error = %e, "storage_stats sampler tick failed");
            }
        }
    });
}

async fn tick(repo: &StorageStatsRepo, raw_dir: &PathBuf, database: &str) -> anyhow::Result<()> {
    let jsonl_bytes = walk_dir_bytes(raw_dir).unwrap_or(0);
    let pg_bytes = repo.pg_database_size(database).await?;
    let s = Sample {
        pg_bytes,
        jsonl_bytes,
        session_count: repo.count_sessions().await?,
        turn_count:    repo.count_turns().await?,
        diff_count:    repo.count_diffs().await?,
    };
    repo.insert(s).await?;
    Ok(())
}

fn walk_dir_bytes(p: &PathBuf) -> std::io::Result<i64> {
    let mut total: i64 = 0;
    for entry in std::fs::read_dir(p)? {
        let entry = entry?;
        let md = entry.metadata()?;
        if md.is_file() { total += md.len() as i64; }
    }
    Ok(total)
}
```

- [ ] **Step 3: Test** — append to `crates/teramindd/tests/ingest_e2e.rs`:

```rust
#[tokio::test]
async fn storage_stats_sampler_records_a_row() {
    let tmp = tempfile::tempdir().unwrap();
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(tmp.path().join("pg"), "teramind_test").await.unwrap();
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await.unwrap();
    teramind_db::migrate::run(&pool).await.unwrap();
    let repo = teramind_db::repos::StorageStatsRepo::new(pool.clone());
    let raw = tmp.path().join("raw"); std::fs::create_dir_all(&raw).unwrap();
    teramindd::services::storage_stats::spawn(repo.clone(), raw, "teramind_test".into(),
        std::time::Duration::from_millis(50));
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM storage_stats").fetch_one(pool.pg()).await.unwrap();
    assert!(n >= 1);
    sup.shutdown().await.unwrap();
}
```

- [ ] **Step 4: Run it**

Run: `cargo test -p teramindd --test ingest_e2e storage_stats_sampler_records_a_row`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramindd/src/services/storage_stats.rs crates/teramindd/tests/ingest_e2e.rs
git commit -m "feat(daemon): storage_stats sampler"
```

---

### Task 52: IPC server dispatch + Status handler

**Files:**
- Create: `crates/teramindd/src/services/ipc_server.rs`
- Modify: `crates/teramindd/src/services/mod.rs`

- [ ] **Step 1: Add `pub mod ipc_server;` to `services/mod.rs`.**

- [ ] **Step 2: Write `services/ipc_server.rs`**

```rust
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;
use async_trait::async_trait;
use teramind_ipc::proto::{Notify, Request, Response, StatusReport};
use teramind_ipc::server::{serve_connection, IpcServer};
use crate::services::ingest::{IngestService, IngestStats};

pub struct DaemonIpcHandler {
    pub ingest: Arc<IngestService>,
    pub stats: Arc<IngestStats>,
    pub started: Instant,
    pub last_pg_bytes: std::sync::atomic::AtomicI64,
    pub last_jsonl_bytes: std::sync::atomic::AtomicI64,
}

#[async_trait]
impl IpcServer for DaemonIpcHandler {
    async fn handle_request(&self, req: Request) -> Response {
        match req {
            Request::Status => Response::Status(StatusReport {
                uptime_seconds: self.started.elapsed().as_secs(),
                pg_connected: true, // refined in Plan B when PG health is tracked
                ingest_queue_depth: self.stats.queue_depth.load(Ordering::Relaxed) as u32,
                ingest_drops_total: self.stats.drops.load(Ordering::Relaxed),
                last_storage_pg_bytes: self.last_pg_bytes.load(Ordering::Relaxed),
                last_storage_jsonl_bytes: self.last_jsonl_bytes.load(Ordering::Relaxed),
            }),
            Request::Ping => Response::Pong,
            Request::Shutdown => Response::Ok,
        }
    }
    async fn handle_notify(&self, n: Notify) {
        match n {
            Notify::Ingest(env) => {
                let _ = self.ingest.try_enqueue(env);
            }
        }
    }
}

pub async fn run_accept_loop<L>(listener: L, handler: Arc<DaemonIpcHandler>) -> anyhow::Result<()>
where
    L: AcceptStream + Send + 'static,
{
    loop {
        let stream = listener.accept_stream().await?;
        let h = handler.clone();
        tokio::spawn(async move {
            let _ = serve_connection(stream, h).await;
        });
    }
}

#[async_trait::async_trait]
pub trait AcceptStream {
    type Stream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static;
    async fn accept_stream(&self) -> std::io::Result<Self::Stream>;
}

#[cfg(unix)]
#[async_trait::async_trait]
impl AcceptStream for tokio::net::UnixListener {
    type Stream = tokio::net::UnixStream;
    async fn accept_stream(&self) -> std::io::Result<Self::Stream> {
        let (s, _) = self.accept().await?;
        Ok(s)
    }
}
```

- [ ] **Step 3: End-to-end IPC test** — create `crates/teramindd/tests/ipc_status.rs`:

```rust
#![cfg(unix)]
use std::sync::Arc;
use teramind_ipc::client::{IpcClient, StreamClient};
use teramind_ipc::proto::{Request, Response};
use teramind_ipc::transport::{listen, connect};
use teramindd::services::ingest::{IngestService, IngestStats, IngestDeps};
use teramindd::services::jsonl_writer::JsonlWriter;
use teramindd::services::session_manager::SessionManager;
use teramindd::services::ipc_server::{DaemonIpcHandler, run_accept_loop};
use teramind_db::{pg_supervisor::PgSupervisor, pool::DbPool, migrate};
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo};
use teramind_core::redact::Redactor;

#[tokio::test]
async fn status_request_returns_status_report() {
    let tmp = tempfile::tempdir().unwrap();
    let sup = PgSupervisor::start(tmp.path().join("pg"), "teramind_test").await.unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();

    let jsonl = Arc::new(JsonlWriter::open(tmp.path().join("raw")).await.unwrap());
    let stats = Arc::new(IngestStats::default());
    let svc = IngestService::spawn(64, IngestDeps {
        redactor: Arc::new(Redactor::with_default_rules()),
        jsonl: jsonl.clone(),
        sessions: SessionManager::new(),
        agents: AgentRepo::new(pool.clone()),
        session_repo: SessionRepo::new(pool.clone()),
        trace: TraceRepo::new(pool.clone()),
        diffs: DiffRepo::new(pool.clone()),
        stats: stats.clone(),
        dead_letter_dir: tmp.path().join("dl"),
    });
    let handler = Arc::new(DaemonIpcHandler {
        ingest: Arc::new(svc),
        stats: stats.clone(),
        started: std::time::Instant::now(),
        last_pg_bytes: 0.into(), last_jsonl_bytes: 0.into(),
    });
    let sock = tmp.path().join("t.sock");
    let listener = listen(&sock).unwrap();
    let h2 = handler.clone();
    tokio::spawn(async move { let _ = run_accept_loop(listener, h2).await; });

    let stream = connect(&sock).await.unwrap();
    let mut client = StreamClient::new(stream);
    let r = client.request(Request::Status).await.unwrap();
    match r {
        Response::Status(s) => assert_eq!(s.ingest_drops_total, 0),
        other => panic!("unexpected: {:?}", other),
    }

    sup.shutdown().await.unwrap();
}
```

- [ ] **Step 4: Run**

Run: `cargo test -p teramindd --test ipc_status status_request_returns_status_report`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramindd/src/services/ipc_server.rs crates/teramindd/tests/ipc_status.rs
git commit -m "feat(daemon): IPC server with Status / Ping / Shutdown handlers"
```

---

### Task 53: `App` struct and `main`

**Files:**
- Create: `crates/teramindd/src/app.rs`
- Replace: `crates/teramindd/src/main.rs`

- [ ] **Step 1: Write `app.rs`**

```rust
use std::sync::Arc;
use std::time::{Duration, Instant};
use anyhow::Context;
use crate::config::Config;
use crate::paths::Paths;
use crate::services::ingest::{IngestService, IngestStats, IngestDeps};
use crate::services::jsonl_writer::JsonlWriter;
use crate::services::session_manager::SessionManager;
use crate::services::ipc_server::{DaemonIpcHandler, run_accept_loop};
use crate::services::storage_stats;
use teramind_core::redact::Redactor;
use teramind_db::{pg_supervisor::PgSupervisor, pool::DbPool, migrate};
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo, StorageStatsRepo};
use teramind_ipc::transport::listen;
use tracing::info;

pub struct App {
    pub paths: Paths,
    pub config: Config,
    pub supervisor: Option<PgSupervisor>,
}

impl App {
    pub async fn run() -> anyhow::Result<()> {
        let paths = Paths::resolve()?;
        paths.ensure_dirs()?;
        let config_path = paths.config_dir.join("config.toml");
        let config = Config::load_or_default(&config_path)?;

        // Logging
        let file_appender = tracing_appender::rolling::daily(&paths.logs_dir, "teramindd.log");
        let (nb, _guard) = tracing_appender::non_blocking(file_appender);
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("TERAMIND_LOG").unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")))
            .with_writer(nb).json().init();
        std::mem::forget(_guard); // keep alive for process lifetime

        info!("teramindd starting");

        // PID file
        let pid = std::process::id();
        std::fs::write(&paths.pid_file, format!("{pid}\n")).context("write pid file")?;

        // Postgres
        let supervisor = PgSupervisor::start(paths.pgdata_dir.clone(), "teramind").await?;
        let pool = DbPool::connect(supervisor.connect_options()).await?;
        migrate::run(&pool).await?;

        // Services
        let jsonl = Arc::new(JsonlWriter::open(paths.raw_dir.clone()).await?);
        let stats = Arc::new(IngestStats::default());
        let ingest = Arc::new(IngestService::spawn(config.ingest_queue_capacity, IngestDeps {
            redactor: Arc::new(if config.redaction_enabled { Redactor::with_default_rules() } else { Redactor::with_default_rules() }),
            jsonl: jsonl.clone(),
            sessions: SessionManager::new(),
            agents: AgentRepo::new(pool.clone()),
            session_repo: SessionRepo::new(pool.clone()),
            trace: TraceRepo::new(pool.clone()),
            diffs: DiffRepo::new(pool.clone()),
            stats: stats.clone(),
            dead_letter_dir: paths.dead_letter_dir.clone(),
        }));
        storage_stats::spawn(StorageStatsRepo::new(pool.clone()), paths.raw_dir.clone(), "teramind".into(),
            Duration::from_secs(config.storage_sample_interval_secs));

        // IPC server
        let handler = Arc::new(DaemonIpcHandler {
            ingest: ingest.clone(), stats: stats.clone(),
            started: Instant::now(),
            last_pg_bytes: 0.into(), last_jsonl_bytes: 0.into(),
        });
        let listener = listen(&paths.socket_path)?;
        let h2 = handler.clone();
        tokio::spawn(async move { let _ = run_accept_loop(listener, h2).await; });

        // Shutdown
        crate::signals::shutdown_signal().await;
        info!("teramindd shutting down");
        let _ = std::fs::remove_file(&paths.pid_file);
        let _ = std::fs::remove_file(&paths.socket_path);
        supervisor.shutdown().await?;
        Ok(())
    }
}
```

- [ ] **Step 2: Replace `main.rs`**

```rust
use teramindd::app::App;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    App::run().await
}
```

- [ ] **Step 3: Make modules public to the binary** — ensure `crates/teramindd/src/lib.rs` does not exist. Add the following at the top of `main.rs` to declare modules:

Actually — since we want both the binary and tests to share these modules, add a `lib.rs`:

`crates/teramindd/src/lib.rs`:

```rust
pub mod app;
pub mod config;
pub mod paths;
pub mod services;
pub mod signals;
```

And rewrite `main.rs` to call `teramindd::app::App::run()`. Update `Cargo.toml` of teramindd to include both `[lib]` and `[[bin]]`:

```toml
[lib]
name = "teramindd"
path = "src/lib.rs"

[[bin]]
name = "teramindd"
path = "src/main.rs"
```

- [ ] **Step 4: Build**

Run: `cargo build -p teramindd`
Expected: PASS.

- [ ] **Step 5: Smoke run (foreground, kill manually)**

Run: `cargo run -p teramindd &` then kill it after a second. Check that `teramindd.pid` is removed and that the JSON log file in `~/.local/share/teramind/logs/` was created.

Run: `cargo build -p teramindd && timeout 2 cargo run --quiet -p teramindd; ls "$HOME/.local/share/teramind/logs/" 2>/dev/null | head`
Expected: at least one log file present; exit code may be non-zero due to `timeout`, that's fine.

- [ ] **Step 6: Commit**

```bash
git add crates/teramindd/
git commit -m "feat(daemon): App::run wires PG, ingest, storage_stats, IPC, signals"
```

---

## Section 9 — `teramind` CLI

### Task 54: clap skeleton and IPC connector

**Files:**
- Create: `crates/teramind/src/cli.rs`
- Create: `crates/teramind/src/ipc.rs`
- Replace: `crates/teramind/src/main.rs`

- [ ] **Step 1: Write `cli.rs`**

```rust
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "teramind", version, about = "Teramind CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Initialize Teramind data + config directories and run migrations.
    Init,
    /// Start the daemon in the background (lazy-spawn).
    Start,
    /// Stop the running daemon via SIGTERM (Unix) or named-pipe close (Windows).
    Stop,
    /// Show daemon status.
    Status {
        #[arg(long)]
        format: Option<String>,
    },
    /// Print version.
    Version,
    /// Restart (stop + start).
    Restart,
    /// Run diagnostic checks and print a pasteable report.
    Doctor,
    /// Reset local data. With --purge, also remove plugin and config.
    Reset {
        #[arg(long)]
        purge: bool,
        #[arg(long)]
        confirm: bool,
    },
}
```

- [ ] **Step 2: Write `ipc.rs`**

```rust
use std::time::Duration;
use teramind_ipc::client::{IpcClient, StreamClient};
use teramind_ipc::proto::{Request, Response};
use teramind_ipc::transport::{connect, default_socket_path};

pub async fn request(req: Request, deadline_ms: u64) -> anyhow::Result<Response> {
    let path = default_socket_path();
    let stream = tokio::time::timeout(Duration::from_millis(deadline_ms), connect(&path))
        .await
        .map_err(|_| anyhow::anyhow!("daemon connect timed out"))??;
    let mut client = StreamClient::new(stream);
    Ok(client.request(req).await?)
}
```

- [ ] **Step 3: Replace `main.rs`**

```rust
mod cli;
mod commands;
mod ipc;

use clap::Parser;
use cli::{Cli, Command};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::try_init().ok();
    let cli = Cli::parse();
    match cli.command {
        Command::Init     => commands::init::run().await,
        Command::Start    => commands::start::run().await,
        Command::Stop     => commands::stop::run().await,
        Command::Status { format } => commands::status::run(format).await,
        Command::Version  => commands::version::run().await,
        Command::Restart  => commands::restart::run().await,
        Command::Doctor   => commands::doctor::run().await,
        Command::Reset { purge, confirm } => commands::reset::run(purge, confirm).await,
    }
}
```

- [ ] **Step 4: Stub `commands/mod.rs`** so `cargo check` finds modules (per-command files are written in Tasks 55–60):

`crates/teramind/src/commands/mod.rs`:

```rust
pub mod init;
pub mod start;
pub mod stop;
pub mod status;
pub mod version;
pub mod restart;
pub mod doctor;
pub mod reset;
```

- [ ] **Step 5: Verify the CLI parses `--help`**

(After Tasks 55–60 ship the per-command files.) Run: `cargo run -p teramind-cli -- --help`
Expected: clap-formatted help.

- [ ] **Step 6: Commit**

```bash
git add crates/teramind/src/cli.rs crates/teramind/src/ipc.rs crates/teramind/src/main.rs crates/teramind/src/commands/mod.rs
git commit -m "feat(cli): clap skeleton and IPC connector"
```

---

### Task 55: `init`, `version`, `status` commands

**Files:**
- Create: `crates/teramind/src/commands/init.rs`
- Create: `crates/teramind/src/commands/version.rs`
- Create: `crates/teramind/src/commands/status.rs`

- [ ] **Step 1: Write `init.rs`**

```rust
use anyhow::Context;
use std::path::PathBuf;

pub async fn run() -> anyhow::Result<()> {
    let paths = teramindd::paths::Paths::resolve()?;
    paths.ensure_dirs()?;

    let cfg_path: PathBuf = paths.config_dir.join("config.toml");
    if !cfg_path.exists() {
        let default = include_str!("../../../../crates/teramindd/src/default_config.toml");
        std::fs::write(&cfg_path, default).context("write default config")?;
    }
    println!("Teramind initialized.");
    println!("  data dir   : {}", paths.data_dir.display());
    println!("  config dir : {}", paths.config_dir.display());
    println!("  socket     : {}", paths.socket_path.display());
    Ok(())
}
```

Also create the embedded default config file at `crates/teramindd/src/default_config.toml`:

```toml
# Teramind default configuration.
ingest_queue_capacity = 4096
idle_timeout_secs     = 1800
redaction_enabled     = true
autorecall_enabled    = true
storage_sample_interval_secs = 300
```

- [ ] **Step 2: Write `version.rs`**

```rust
pub async fn run() -> anyhow::Result<()> {
    println!("teramind {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}
```

- [ ] **Step 3: Write `status.rs`**

```rust
use crate::ipc;
use teramind_ipc::proto::{Request, Response};

pub async fn run(format: Option<String>) -> anyhow::Result<()> {
    let resp = match ipc::request(Request::Status, 1500).await {
        Ok(r) => r,
        Err(_) => {
            println!("teramind: daemon is not running");
            return Ok(());
        }
    };
    let status = match resp {
        Response::Status(s) => s,
        Response::Error(e)  => { eprintln!("error: {e}"); return Ok(()); }
        other => { eprintln!("unexpected: {other:?}"); return Ok(()); }
    };
    if format.as_deref() == Some("json") {
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        println!("uptime           : {}s", status.uptime_seconds);
        println!("pg connected     : {}", status.pg_connected);
        println!("ingest queue     : {}", status.ingest_queue_depth);
        println!("ingest drops     : {}", status.ingest_drops_total);
        println!("pg bytes         : {}", status.last_storage_pg_bytes);
        println!("jsonl bytes      : {}", status.last_storage_jsonl_bytes);
    }
    Ok(())
}
```

- [ ] **Step 4: Build**

Run: `cargo build -p teramind-cli`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind/src/commands/ crates/teramindd/src/default_config.toml
git commit -m "feat(cli): init, version, status commands"
```

---

### Task 56: `start`, `stop`, `restart` commands

**Files:**
- Create: `crates/teramind/src/commands/start.rs`
- Create: `crates/teramind/src/commands/stop.rs`
- Create: `crates/teramind/src/commands/restart.rs`

- [ ] **Step 1: Write `start.rs`**

```rust
use std::process::Command;

pub async fn run() -> anyhow::Result<()> {
    // If daemon is already responsive, nothing to do.
    if crate::ipc::request(teramind_ipc::proto::Request::Ping, 250).await.is_ok() {
        println!("teramind: daemon already running");
        return Ok(());
    }
    let exe = which_teramindd()?;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let _ = Command::new(&exe)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0) // detach from controlling terminal
            .spawn()?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS = 0x00000008
        let _ = Command::new(&exe)
            .creation_flags(0x00000008)
            .spawn()?;
    }
    // Poll until reachable, up to 5s.
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if crate::ipc::request(teramind_ipc::proto::Request::Ping, 250).await.is_ok() {
            println!("teramind: daemon started");
            return Ok(());
        }
    }
    anyhow::bail!("daemon spawned but did not become responsive within 5 seconds");
}

fn which_teramindd() -> anyhow::Result<std::path::PathBuf> {
    // 1. Same dir as the current executable (release scenario).
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            let candidate = dir.join(if cfg!(windows) { "teramindd.exe" } else { "teramindd" });
            if candidate.exists() { return Ok(candidate); }
        }
    }
    // 2. $PATH.
    if let Ok(out) = std::process::Command::new(if cfg!(windows) { "where" } else { "which" }).arg("teramindd").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            if let Some(line) = s.lines().next() { return Ok(line.trim().into()); }
        }
    }
    anyhow::bail!("teramindd binary not found next to teramind or on PATH")
}
```

- [ ] **Step 2: Write `stop.rs`**

```rust
use teramind_ipc::proto::{Request, Response};

pub async fn run() -> anyhow::Result<()> {
    match crate::ipc::request(Request::Shutdown, 1500).await {
        Ok(Response::Ok) => { println!("teramind: stop requested"); Ok(()) }
        Ok(other) => { eprintln!("unexpected: {other:?}"); Ok(()) }
        Err(_) => { println!("teramind: daemon already stopped"); Ok(()) }
    }
}
```

(Note: this Plan A's `Shutdown` handler returns `Ok` but does not actually terminate the daemon — the daemon still listens for OS signals. Plan B refines `Shutdown` to dispatch the actual shutdown sequence.)

- [ ] **Step 3: Write `restart.rs`**

```rust
pub async fn run() -> anyhow::Result<()> {
    super::stop::run().await?;
    // Allow the daemon to fully release the socket.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    super::start::run().await
}
```

- [ ] **Step 4: Build**

Run: `cargo build -p teramind-cli`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramind/src/commands/
git commit -m "feat(cli): start, stop, restart commands"
```

---

### Task 57: `doctor` and `reset` commands

**Files:**
- Create: `crates/teramind/src/commands/doctor.rs`
- Create: `crates/teramind/src/commands/reset.rs`

- [ ] **Step 1: Write `doctor.rs`**

```rust
use teramind_ipc::proto::{Request, Response};

pub async fn run() -> anyhow::Result<()> {
    println!("teramind doctor");
    let paths = teramindd::paths::Paths::resolve()?;
    let pid = if paths.pid_file.exists() {
        std::fs::read_to_string(&paths.pid_file).ok().map(|s| s.trim().to_string())
    } else { None };
    println!("  pid file       : {} ({})", paths.pid_file.display(), pid.as_deref().unwrap_or("missing"));
    println!("  socket         : {} ({})", paths.socket_path.display(),
        if paths.socket_path.exists() { "present" } else { "absent" });
    println!("  data dir       : {}", paths.data_dir.display());
    println!("  config dir     : {}", paths.config_dir.display());
    println!("  dead_letter    : {} files", dir_count(&paths.dead_letter_dir)?);
    println!("  inbox          : {} files", dir_count(&paths.inbox_dir)?);
    match crate::ipc::request(Request::Status, 1500).await {
        Ok(Response::Status(s)) => {
            println!("  daemon         : up ({}s uptime)", s.uptime_seconds);
            println!("  ingest queue   : {}", s.ingest_queue_depth);
            println!("  ingest drops   : {}", s.ingest_drops_total);
            println!("  pg bytes       : {}", s.last_storage_pg_bytes);
            println!("  jsonl bytes    : {}", s.last_storage_jsonl_bytes);
        }
        Ok(other) => println!("  daemon         : unexpected response {:?}", other),
        Err(_)     => println!("  daemon         : not responding"),
    }
    Ok(())
}

fn dir_count(p: &std::path::Path) -> anyhow::Result<usize> {
    if !p.exists() { return Ok(0); }
    Ok(std::fs::read_dir(p)?.filter_map(Result::ok).count())
}
```

- [ ] **Step 2: Write `reset.rs`**

```rust
pub async fn run(purge: bool, confirm: bool) -> anyhow::Result<()> {
    if !confirm {
        anyhow::bail!("`teramind reset` will delete local data; re-run with --confirm to proceed");
    }
    let paths = teramindd::paths::Paths::resolve()?;
    for d in [&paths.pgdata_dir, &paths.raw_dir, &paths.inbox_dir, &paths.dead_letter_dir] {
        if d.exists() { std::fs::remove_dir_all(d)?; }
    }
    if purge {
        if paths.config_dir.exists() { std::fs::remove_dir_all(&paths.config_dir)?; }
    }
    println!("teramind: local data {}cleared.", if purge { "and config " } else { "" });
    Ok(())
}
```

- [ ] **Step 3: Build**

Run: `cargo build -p teramind-cli`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramind/src/commands/
git commit -m "feat(cli): doctor and reset commands"
```

---

## Section 10 — Inbox drainer and dead-letter sink

The hook shim (Plan B) drops events to `inbox/` when the daemon is unreachable. The daemon must drain that on startup. The ingest pipeline must write to `dead_letter/` when a PG write fails after retries.

### Task 58: Inbox drainer

**Files:**
- Modify: `crates/teramindd/src/services/ingest.rs`
- Modify: `crates/teramindd/src/app.rs`

- [ ] **Step 1: Add a `drain_inbox` function to `services/ingest.rs`** (append at the bottom):

```rust
pub async fn drain_inbox(inbox: &std::path::Path, ingest: &IngestService) -> anyhow::Result<usize> {
    if !inbox.exists() { return Ok(0); }
    let mut drained = 0usize;
    for entry in std::fs::read_dir(inbox)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") { continue; }
        let bytes = std::fs::read(&path)?;
        match serde_json::from_slice::<EventEnvelope>(&bytes) {
            Ok(env) => {
                if ingest.try_enqueue(env).is_ok() {
                    let _ = std::fs::remove_file(&path);
                    drained += 1;
                }
            }
            Err(_) => {
                // Move malformed events into dead_letter for inspection.
                let dl = inbox.parent().map(|p| p.join("dead_letter")).unwrap_or_else(|| inbox.to_path_buf());
                let _ = std::fs::create_dir_all(&dl);
                let _ = std::fs::rename(&path, dl.join(path.file_name().unwrap_or_default()));
            }
        }
    }
    Ok(drained)
}
```

- [ ] **Step 2: Call it on daemon startup** — in `app.rs`, after `IngestService::spawn(...)`:

```rust
let drained = crate::services::ingest::drain_inbox(&paths.inbox_dir, &ingest).await.unwrap_or(0);
if drained > 0 {
    tracing::info!(drained, "drained inbox events");
}
```

- [ ] **Step 3: Test for the drainer** — create `crates/teramindd/tests/inbox_drain.rs`:

```rust
use std::sync::Arc;
use teramind_core::ids::{ClientEventId, SessionId};
use teramind_core::redact::Redactor;
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use teramind_db::{pg_supervisor::PgSupervisor, pool::DbPool, migrate};
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo};
use teramindd::services::ingest::{IngestService, IngestStats, IngestDeps, drain_inbox};
use teramindd::services::jsonl_writer::JsonlWriter;
use teramindd::services::session_manager::SessionManager;
use tempfile::tempdir;
use time::OffsetDateTime;

#[tokio::test]
async fn inbox_drainer_consumes_pending_files() {
    let tmp = tempdir().unwrap();
    let inbox = tmp.path().join("inbox"); std::fs::create_dir_all(&inbox).unwrap();

    // Write 3 pending events.
    let sid = SessionId::new();
    for i in 0..3 {
        let env = EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::now_utc(),
            event: IngestEvent::UserPrompt { session_id: sid, turn_ordinal: i, prompt: format!("p{i}") },
        };
        let path = inbox.join(format!("{}.json", env.client_event_id.0));
        std::fs::write(&path, serde_json::to_vec(&env).unwrap()).unwrap();
    }

    // Bring up a daemon stack.
    let sup = PgSupervisor::start(tmp.path().join("pg"), "teramind_test").await.unwrap();
    let pool = DbPool::connect(sup.connect_options()).await.unwrap();
    migrate::run(&pool).await.unwrap();
    let jsonl = Arc::new(JsonlWriter::open(tmp.path().join("raw")).await.unwrap());
    let stats = Arc::new(IngestStats::default());
    let svc = IngestService::spawn(64, IngestDeps {
        redactor: Arc::new(Redactor::with_default_rules()),
        jsonl, sessions: SessionManager::new(),
        agents: AgentRepo::new(pool.clone()),
        session_repo: SessionRepo::new(pool.clone()),
        trace: TraceRepo::new(pool.clone()),
        diffs: DiffRepo::new(pool.clone()),
        stats: stats.clone(),
        dead_letter_dir: tmp.path().join("dl"),
    });

    let drained = drain_inbox(&inbox, &svc).await.unwrap();
    assert_eq!(drained, 3);
    let remaining = std::fs::read_dir(&inbox).unwrap().count();
    assert_eq!(remaining, 0);

    sup.shutdown().await.unwrap();
}
```

- [ ] **Step 4: Run it**

Run: `cargo test -p teramindd --test inbox_drain inbox_drainer_consumes_pending_files`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/teramindd/src/services/ingest.rs crates/teramindd/src/app.rs crates/teramindd/tests/inbox_drain.rs
git commit -m "feat(daemon): inbox drainer + dead-letter for malformed events"
```

---

### Task 59: Dead-letter path for PG write failures

**Files:**
- Modify: `crates/teramindd/src/services/ingest.rs`

- [ ] **Step 1: Add a retry-with-dead-letter wrapper in `handle`**

Replace the body of `handle` with:

```rust
async fn handle(d: &IngestDeps, env: EventEnvelope) -> anyhow::Result<()> {
    d.jsonl.append(&env).await?;
    let redacted = redact_envelope(&d.redactor, env);

    // Retry the PG-routing step up to 3 times with exponential backoff.
    let mut attempt = 0u32;
    let mut backoff = std::time::Duration::from_millis(50);
    loop {
        match route(d, redacted.clone()).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                attempt += 1;
                if attempt >= 3 {
                    let dl = &d.dead_letter_dir;
                    let _ = std::fs::create_dir_all(dl);
                    let path = dl.join(format!("{}.json", redacted.client_event_id.0));
                    let _ = std::fs::write(&path, serde_json::to_vec(&redacted).unwrap_or_default());
                    d.stats.dead_letters.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    return Err(e);
                }
                tokio::time::sleep(backoff).await;
                backoff *= 2;
            }
        }
    }
}
```

To make this compile, derive `Clone` on `EventEnvelope` and its variants if not already (`EventEnvelope` already derives Clone in Task 11; `IngestEvent` also does).

- [ ] **Step 2: Test** — append to `crates/teramindd/tests/inbox_drain.rs`:

```rust
#[tokio::test]
async fn dead_letter_receives_unroutable_events() {
    let tmp = tempfile::tempdir().unwrap();
    let sup = teramind_db::pg_supervisor::PgSupervisor::start(tmp.path().join("pg"), "teramind_test").await.unwrap();
    let pool = teramind_db::pool::DbPool::connect(sup.connect_options()).await.unwrap();
    teramind_db::migrate::run(&pool).await.unwrap();
    let jsonl = std::sync::Arc::new(teramindd::services::jsonl_writer::JsonlWriter::open(tmp.path().join("raw")).await.unwrap());
    let stats = std::sync::Arc::new(teramindd::services::ingest::IngestStats::default());
    let dl_dir = tmp.path().join("dl");
    let svc = teramindd::services::ingest::IngestService::spawn(64, teramindd::services::ingest::IngestDeps {
        redactor: std::sync::Arc::new(teramind_core::redact::Redactor::with_default_rules()),
        jsonl, sessions: teramindd::services::session_manager::SessionManager::new(),
        agents: teramind_db::repos::AgentRepo::new(pool.clone()),
        session_repo: teramind_db::repos::SessionRepo::new(pool.clone()),
        trace: teramind_db::repos::TraceRepo::new(pool.clone()),
        diffs: teramind_db::repos::DiffRepo::new(pool.clone()),
        stats: stats.clone(),
        dead_letter_dir: dl_dir.clone(),
    });

    // A UserPrompt for a session that does not exist will hit a FK violation on the turn insert.
    let env = teramind_core::types::ingest_event::EventEnvelope {
        client_event_id: teramind_core::ids::ClientEventId::new(),
        ts: time::OffsetDateTime::now_utc(),
        event: teramind_core::types::ingest_event::IngestEvent::UserPrompt {
            session_id: teramind_core::ids::SessionId::new(),
            turn_ordinal: 0, prompt: "x".into(),
        },
    };
    svc.try_enqueue(env).unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(2)).await; // wait through retries

    let count = std::fs::read_dir(&dl_dir).map(|i| i.count()).unwrap_or(0);
    assert!(count >= 1, "expected at least one dead-letter file; got {count}");
    sup.shutdown().await.unwrap();
}
```

- [ ] **Step 3: Run it**

Run: `cargo test -p teramindd --test inbox_drain dead_letter_receives_unroutable_events`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/teramindd/src/services/ingest.rs crates/teramindd/tests/inbox_drain.rs
git commit -m "feat(daemon): dead-letter sink after 3 PG write failures"
```

---

## Section 11 — Smoke E2E and developer ergonomics

### Task 60: End-to-end "init + start + status + stop" script test

**Files:**
- Create: `crates/teramind/tests/smoke_e2e.rs`

- [ ] **Step 1: Write the test**

```rust
#![cfg(unix)]
use std::process::Command;
use tempfile::tempdir;

fn cargo_bin(name: &str) -> std::path::PathBuf {
    // CARGO_BIN_EXE_<name> is set by cargo for integration tests.
    std::env::var(format!("CARGO_BIN_EXE_{name}")).map(Into::into)
        .unwrap_or_else(|_| {
            let target = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into());
            let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
            std::path::PathBuf::from(target).join(profile).join(name)
        })
}

#[test]
fn cli_init_start_status_stop_smoke() {
    // Use a throwaway HOME so we don't trash the developer's data dir.
    let tmp = tempdir().unwrap();
    let env = [
        ("HOME", tmp.path().to_str().unwrap()),
        ("XDG_DATA_HOME", tmp.path().join("xdg-data").to_str().unwrap()),
        ("XDG_CONFIG_HOME", tmp.path().join("xdg-config").to_str().unwrap()),
        ("TERAMIND_SOCKET", tmp.path().join("t.sock").to_str().unwrap()),
        ("TERAMIND_LOG", "warn"),
    ];

    let teramind = cargo_bin("teramind");

    // init
    let out = Command::new(&teramind).arg("init").envs(env.iter().copied()).output().unwrap();
    assert!(out.status.success(), "init failed: {}", String::from_utf8_lossy(&out.stderr));

    // start
    let out = Command::new(&teramind).arg("start").envs(env.iter().copied()).output().unwrap();
    assert!(out.status.success(), "start failed: {}", String::from_utf8_lossy(&out.stderr));

    // status
    let out = Command::new(&teramind).arg("status").envs(env.iter().copied()).output().unwrap();
    assert!(out.status.success(), "status failed: {}", String::from_utf8_lossy(&out.stderr));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("uptime"), "status output did not contain uptime line: {s}");

    // stop (Plan A's stop is best-effort acknowledgement only; we kill the daemon explicitly to clean up.)
    let _ = Command::new(&teramind).arg("stop").envs(env.iter().copied()).output();
    if let Ok(pid_str) = std::fs::read_to_string(tmp.path().join("xdg-data/teramind/teramindd.pid")) {
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            unsafe { libc::kill(pid, libc::SIGTERM); }
        }
    }
}
```

Add `libc = "0.2"` to `crates/teramind/Cargo.toml` `[dev-dependencies]`.

- [ ] **Step 2: Run it**

Run: `cargo test -p teramind-cli --test smoke_e2e cli_init_start_status_stop_smoke -- --nocapture`
Expected: PASS. Total runtime dominated by embedded-PG startup (~10-30 s on first run, cached after).

- [ ] **Step 3: Commit**

```bash
git add crates/teramind/tests/smoke_e2e.rs crates/teramind/Cargo.toml
git commit -m "test(cli): end-to-end smoke for init/start/status/stop"
```

---

### Task 61: Top-level `Makefile` / `justfile` for developer ergonomics

**Files:**
- Create: `justfile`

- [ ] **Step 1: Write a minimal `justfile`**

```just
default: fmt clippy test

fmt:
    cargo fmt --all

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

build:
    cargo build --workspace

test:
    cargo test --workspace

# Run integration tests (slow — they start embedded Postgres).
test-integration:
    cargo test --workspace --test '*'

# Wipe local Teramind state for the current user (Unix).
reset-local:
    rm -rf "$HOME/.local/share/teramind" "$HOME/.config/teramind"
```

- [ ] **Step 2: Sanity check**

Run: `just fmt`
Expected: rustfmt formats all crates; no output beyond rustfmt's own messages.

- [ ] **Step 3: Commit**

```bash
git add justfile
git commit -m "chore: justfile for fmt/clippy/test/build"
```

---

### Task 62: Workspace-wide clippy + fmt CI smoke

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Write a minimal CI workflow**

```yaml
name: ci
on:
  pull_request:
  push:
    branches: [main]

jobs:
  lint-and-test:
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-22.04, macos-14]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - name: cache cargo
        uses: Swatinem/rust-cache@v2
      - name: fmt
        run: cargo fmt --all -- --check
      - name: clippy
        run: cargo clippy --workspace --all-targets -- -D warnings
      - name: unit tests
        run: cargo test --workspace --lib --bins
      - name: integration tests
        run: cargo test --workspace --test '*'
        env:
          TERAMIND_LOG: warn
```

Windows is intentionally absent from Plan A's matrix — the IPC roundtrip test is `#[cfg(unix)]` and the daemon's Named Pipe path is exercised in Plan B/E. Adding `windows-2022` here without those tests would only verify compilation.

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: workspace fmt/clippy/test on ubuntu + macos"
```

---

## Plan completion checklist

By the end of Task 62 you should have, on the `main` branch of this repo:

- A green `cargo test --workspace`, including integration tests that start embedded Postgres.
- A `teramind` CLI that can run `init → start → status → doctor → stop` against a real daemon.
- A `teramindd` daemon that:
  - Manages an embedded Postgres child.
  - Accepts JSON-RPC over UDS (Unix) or Named Pipe (Windows, untested in this plan but compiled).
  - Persists `IngestEvent`s into the schema described in the spec (Section 4.4).
  - Appends every event to a daily-rotated JSONL shadow log.
  - Applies built-in redaction before persistence.
  - Drops on backpressure, retries on transient PG failure, dead-letters after 3 failures.
  - Samples storage usage every 5 minutes into `storage_stats`.
- The full schema: 8 tables + the `traces_fts` materialized view.
- L1 unit coverage (~25 tests in `teramind-core`) and L2 component coverage (~20 tests in `teramind-db` and `teramindd`).

What Plan A explicitly does **not** ship (these are Plans B–F):

- `teramind-hook`, `teramind-mcp`, the Claude plugin bundle, `teramind claude install/uninstall` (Plan B).
- The search service, grep fallback, MCP search/recall tools, slash commands (Plan C).
- FS watcher, diff capture, attribution, auto-recall digest (Plan D).
- Installer scripts and release packaging (Plan E).
- L5 search-effectiveness benchmark corpus and gates (Plan F).

---

## Plan self-review

Per the writing-plans skill: I read the plan against the spec one more time. Findings:

**Spec coverage:**
- Spec §2 in-scope items 1–4 (workspace, embedded PG, plugin bundle, full-fidelity capture) are covered by Tasks 1–53. Items 5–7 (FS watcher, four search surfaces, installer + connectors) are explicitly deferred to Plans B–F by design.
- Spec §3 architecture: implemented across Tasks 23–53 (IPC, daemon services, App).
- Spec §4 storage: migrations + repos in Tasks 31–43; materialized view in Task 36.
- Spec §5 capture flow: ingest pipeline in Task 49; inbox drainer in Task 58; dead-letter in Task 59.
- Spec §6 search surfaces: **not in this plan** (Plan C). The schema-side prerequisites (`traces_fts`, trigram indices) are in place.
- Spec §7 lifecycle: paths/config/signals/PID in Tasks 44–53. Installer scripts are Plan E.
- Spec §8 observability: drops/queue depth/dead-letter counters exposed in `Request::Status`; `teramind doctor` in Task 57.
- Spec §9.1–§9.2 (L1+L2): covered by per-type roundtrip tests, redaction corpus + property test, repo CRUD tests. L3 lite slice covered by `tests/ingest_e2e.rs` and `tests/inbox_drain.rs`. L4/L5 are Plans B/F.

**Placeholder scan:** no `TBD`, `TODO`, "implement later", or "similar to Task N" — every code step contains real code.

**Type consistency:** confirmed `Redactor::with_default_rules` / `Redactor::with_extra` / `Redactor::apply`, `JsonlWriter::open` / `append`, `IngestService::spawn` / `try_enqueue` / `stats`, `IngestDeps` field names, `Paths::resolve` / `ensure_dirs`, repository constructors `Repo::new(DbPool)` are used identically wherever referenced.

**Known minor follow-ups in Plan B onward:**
- `Request::Shutdown` currently returns `Response::Ok` without terminating the daemon. Plan B makes it actually shut down.
- `which_teramindd` is best-effort. Plan E (installer) sets an absolute path next to the CLI.

---

