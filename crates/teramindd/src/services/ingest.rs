use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;
use teramind_core::redact::Redactor;
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use teramind_db::repos::{AgentRepo, DiffRepo, SessionRepo, TraceRepo};
use teramind_db::repos::session::NewSession;
use crate::services::jsonl_writer::JsonlWriter;
use crate::services::session_manager::{ActiveSession, SessionManager};
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
    d.jsonl.append(&env).await?;
    let redacted = redact_envelope(&d.redactor, env);
    // Plan A's basic version (Task 49) — Task 59 enhances with retry+dead-letter.
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
            let n = NewSession {
                agent_id: agent.id,
                agent_session_id: agent_session_id.as_deref(),
                cwd: &cwd,
                project_id: None,
                parent_session_id: None,
                git_head: git_head.as_deref(),
                git_branch: git_branch.as_deref(),
                os: &os, hostname: &hostname, user_login: &user_login,
                started_at: ts,
            };
            let sid = if session_id.0 != uuid::Uuid::nil() {
                d.session_repo.insert_with_id(session_id, n).await?
            } else {
                d.session_repo.insert(n).await?
            };
            d.sessions.start(ActiveSession {
                session_id: sid, cwd: cwd.clone(), agent_kind, started_at: ts, last_activity: ts, last_turn_id: None
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
                let dl = inbox.parent().map(|p| p.join("dead_letter")).unwrap_or_else(|| inbox.to_path_buf());
                let _ = std::fs::create_dir_all(&dl);
                let _ = std::fs::rename(&path, dl.join(path.file_name().unwrap_or_default()));
            }
        }
    }
    Ok(drained)
}
