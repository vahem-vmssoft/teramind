use crate::services::jsonl_writer::JsonlWriter;
use crate::services::session_manager::{ActiveSession, SessionManager};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use teramind_core::redact::Redactor;
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use teramind_db::repos::session::NewSession;
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo};
use tokio::sync::mpsc;
use tracing::warn;

#[derive(Default)]
pub struct IngestStats {
    pub drops: AtomicU64,
    pub queue_depth: AtomicU64,
    pub pg_write_failures: AtomicU64,
    pub dead_letters: AtomicU64,
    pub fs_watcher_gaps: AtomicU64,
}

pub struct IngestService {
    tx: mpsc::Sender<EventEnvelope>,
    stats: Arc<IngestStats>,
}

/// Subset of IngestDeps that the dispatch fn actually needs. Used by both
/// the daemon (which wraps it in IngestDeps) and the sync server (which
/// constructs it directly).
#[derive(Clone)]
pub struct RouteDeps {
    pub sessions: crate::services::session_manager::SessionManager,
    pub agents: teramind_db::repos::AgentRepo,
    pub session_repo: teramind_db::repos::SessionRepo,
    pub trace: teramind_db::repos::TraceRepo,
    pub diffs: teramind_db::repos::DiffRepo,
    pub fs_registry: std::sync::Arc<crate::services::fs_watcher::WatchRegistry>,
    pub write_tool_ring: crate::services::write_tool_ring::WriteToolRing,
}

impl From<&IngestDeps> for RouteDeps {
    fn from(d: &IngestDeps) -> Self {
        Self {
            sessions: d.sessions.clone(),
            agents: d.agents.clone(),
            session_repo: d.session_repo.clone(),
            trace: d.trace.clone(),
            diffs: d.diffs.clone(),
            fs_registry: d.fs_registry.clone(),
            write_tool_ring: d.write_tool_ring.clone(),
        }
    }
}

/// `(user_id, device_id)` annotation for server-side ingest. The daemon
/// passes `None`; the server passes `Some(...)`.
#[derive(Debug, Clone, Copy)]
pub struct IngestAuth {
    pub user_id: uuid::Uuid,
    pub device_id: uuid::Uuid,
}

/// Public dispatch entry point. Same body as the old `route()` but uses
/// `RouteDeps` + `IngestAuth`. The daemon path passes `auth = None`.
pub async fn route_with_deps(
    d: &RouteDeps,
    env: teramind_core::types::ingest_event::EventEnvelope,
    auth: Option<IngestAuth>,
) -> anyhow::Result<()> {
    route_inner(d, env, auth).await
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
    pub write_tool_ring: crate::services::write_tool_ring::WriteToolRing,
    pub fs_registry: std::sync::Arc<crate::services::fs_watcher::WatchRegistry>,
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
                    stats_for_loop
                        .pg_write_failures
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        Self { tx, stats }
    }

    #[allow(clippy::result_large_err)] // returning the rejected envelope by value is intentional — callers may re-route it
    pub fn try_enqueue(&self, env: EventEnvelope) -> Result<(), EventEnvelope> {
        match self.tx.try_send(env) {
            Ok(_) => {
                self.stats.queue_depth.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(mpsc::error::TrySendError::Full(env))
            | Err(mpsc::error::TrySendError::Closed(env)) => {
                self.stats.drops.fetch_add(1, Ordering::Relaxed);
                Err(env)
            }
        }
    }

    pub fn stats(&self) -> Arc<IngestStats> {
        self.stats.clone()
    }
}

async fn handle(d: &IngestDeps, env: EventEnvelope) -> anyhow::Result<()> {
    // Redaction must run before ANY persistence (Postgres + the JSONL shadow
    // log). The shadow log feeds both `grep_fallback` search results and the
    // team-sync forwarder, so an unredacted append here would leak secrets
    // through both paths.
    let redacted = redact_envelope(&d.redactor, env);
    d.jsonl.append(&redacted).await?;

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
                    let _ =
                        std::fs::write(&path, serde_json::to_vec(&redacted).unwrap_or_default());
                    d.stats
                        .dead_letters
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    return Err(e);
                }
                tokio::time::sleep(backoff).await;
                backoff *= 2;
            }
        }
    }
}

fn redact_envelope(r: &Redactor, mut env: EventEnvelope) -> EventEnvelope {
    use IngestEvent::*;
    env.event = match env.event {
        UserPrompt {
            session_id,
            turn_ordinal,
            prompt,
            turn_id,
        } => UserPrompt {
            session_id,
            turn_ordinal,
            prompt: r.apply(&prompt),
            turn_id,
        },
        ToolCallStart {
            turn_id,
            tool_call_id,
            ordinal,
            name,
            input,
        } => ToolCallStart {
            turn_id,
            tool_call_id,
            ordinal,
            name,
            input: serde_json::from_str(&r.apply(&input.to_string())).unwrap_or(input),
        },
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
        AssistantTurn {
            turn_id,
            assistant_text,
            thinking,
            model,
            input_tokens,
            output_tokens,
        } => AssistantTurn {
            turn_id,
            assistant_text: r.apply(&assistant_text),
            thinking: thinking.map(|t| r.apply(&t)),
            model,
            input_tokens,
            output_tokens,
        },
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
        other => other,
    };
    env
}

async fn route(d: &IngestDeps, env: EventEnvelope) -> anyhow::Result<()> {
    let rd: RouteDeps = d.into();
    route_inner(&rd, env, None).await
}

async fn route_inner(
    d: &RouteDeps,
    env: EventEnvelope,
    auth: Option<IngestAuth>,
) -> anyhow::Result<()> {
    use IngestEvent::*;
    let ts = env.ts;
    match env.event {
        SessionStart {
            session_id,
            agent_session_id,
            agent_kind,
            cwd,
            os,
            hostname,
            user_login,
            git_head,
            git_branch,
        } => {
            let agent = d.agents.upsert(&agent_kind, None).await?;
            let n = NewSession {
                agent_id: agent.id,
                agent_session_id: agent_session_id.as_deref(),
                cwd: &cwd,
                project_id: None,
                parent_session_id: None,
                git_head: git_head.as_deref(),
                git_branch: git_branch.as_deref(),
                os: &os,
                hostname: &hostname,
                user_login: &user_login,
                started_at: ts,
                user_id: auth.map(|a| teramind_core::ids::UserId(a.user_id)),
                device_id: auth.map(|a| teramind_core::ids::DeviceId(a.device_id)),
            };
            let sid = if session_id.0 != uuid::Uuid::nil() {
                d.session_repo.insert_with_id(session_id, n).await?
            } else {
                d.session_repo.insert(n).await?
            };
            d.sessions
                .start(ActiveSession {
                    session_id: sid,
                    cwd: cwd.clone(),
                    agent_kind,
                    started_at: ts,
                    last_activity: ts,
                    last_turn_id: None,
                })
                .await;
            // Start watching this cwd; per-cwd refcount in the registry
            // handles duplicate sessions in the same directory.
            if let Err(e) = d
                .fs_registry
                .register(std::path::PathBuf::from(&cwd), sid)
                .await
            {
                warn!(error = %e, cwd, "fs_watcher: register failed");
            }
        }
        UserPrompt {
            session_id,
            turn_ordinal,
            prompt,
            turn_id,
        } => {
            let _ = match turn_id {
                Some(tid) => {
                    d.trace
                        .upsert_turn_with_id(tid, session_id, turn_ordinal, ts, Some(&prompt))
                        .await?
                }
                None => {
                    d.trace
                        .upsert_turn(session_id, turn_ordinal, ts, Some(&prompt))
                        .await?
                }
            };
            d.sessions.touch(session_id, ts, None).await;
        }
        ToolCallStart {
            turn_id,
            tool_call_id,
            ordinal,
            name,
            input,
        } => match tool_call_id {
            Some(id) => {
                d.trace
                    .insert_tool_call_start_with_id(id, turn_id, ordinal, &name, &input, ts)
                    .await?;
            }
            None => {
                let _ = d
                    .trace
                    .insert_tool_call_start(turn_id, ordinal, &name, &input, ts)
                    .await?;
            }
        },
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
            if let (Some(sid), Some(tid), Some(name)) = (session_id, turn_id, tool_name.as_deref())
            {
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
        AssistantTurn {
            turn_id,
            assistant_text,
            thinking,
            model,
            input_tokens,
            output_tokens,
        } => {
            d.trace
                .finalize_turn(
                    turn_id,
                    ts,
                    Some(&assistant_text),
                    thinking.as_deref(),
                    model.as_deref(),
                    input_tokens,
                    output_tokens,
                )
                .await?;
        }
        SessionEnd { session_id, reason } => {
            d.session_repo.end(session_id, ts, &reason).await?;
            if let Some(active) = d.sessions.end(session_id).await {
                d.fs_registry
                    .unregister(std::path::Path::new(&active.cwd), session_id)
                    .await;
            }
        }
        PreCompact { session_id } => {
            d.session_repo
                .append_metadata(
                    session_id,
                    "pre_compact_at",
                    serde_json::Value::String(ts.to_string()),
                )
                .await?;
        }
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
    }
    Ok(())
}

pub async fn drain_inbox(inbox: &std::path::Path, ingest: &IngestService) -> anyhow::Result<usize> {
    if !inbox.exists() {
        return Ok(0);
    }
    let mut drained = 0usize;
    for entry in std::fs::read_dir(inbox)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let bytes = std::fs::read(&path)?;
        match serde_json::from_slice::<EventEnvelope>(&bytes) {
            Ok(env) => {
                if ingest.try_enqueue(env).is_ok() {
                    let _ = std::fs::remove_file(&path);
                    drained += 1;
                }
            }
            Err(_) => {
                let dl = inbox
                    .parent()
                    .map(|p| p.join("dead_letter"))
                    .unwrap_or_else(|| inbox.to_path_buf());
                let _ = std::fs::create_dir_all(&dl);
                let _ = std::fs::rename(&path, dl.join(path.file_name().unwrap_or_default()));
            }
        }
    }
    Ok(drained)
}
