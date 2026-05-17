//! Tail-JSONL forwarder. Ships captured events from local JSONL to the
//! central sync server via POST /v1/ingest with DPoP-signed requests.

use crate::services::decision_cache::{DecisionCache, ShareDecision};
use crate::services::sync_offset::SyncOffset;
use anyhow::{Context, Result};
use ed25519_dalek::SigningKey;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use teramind_core::dpop::{body_hash_hex, sign, token_hash_hex, ProofClaims};
use teramind_core::ids::SessionId;
use teramind_core::team::TeamConfig;
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use time::OffsetDateTime;
use tokio::io::AsyncBufReadExt;
use tracing::{info, warn};

pub struct TeamSyncDeps {
    pub team_cfg: Arc<TeamConfig>,
    pub signing_key: Arc<SigningKey>,
    pub raw_dir: PathBuf,
    pub cache: Arc<DecisionCache>,
    pub poll_interval: Duration,
    pub batch_size: usize,
    pub max_attempts: u32,
}

pub struct TeamSync {
    _handle: tokio::task::JoinHandle<()>,
}

impl TeamSync {
    pub fn spawn(deps: TeamSyncDeps) -> Self {
        let handle = tokio::spawn(async move {
            run_loop(deps).await;
        });
        Self { _handle: handle }
    }
}

async fn run_loop(deps: TeamSyncDeps) {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .build()
        .expect("reqwest client");

    loop {
        match tick(&deps, &client).await {
            Ok(true) => { /* shipped something; loop tight */ }
            Ok(false) => {
                tokio::time::sleep(deps.poll_interval).await;
            }
            Err(e) => {
                warn!(error = %e, "team_sync tick error");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

async fn tick(deps: &TeamSyncDeps, client: &reqwest::Client) -> Result<bool> {
    let offset = SyncOffset::load(&deps.raw_dir)?;
    let (path, start_byte) = select_jsonl_file(deps, &offset)?;
    let Some(path) = path else {
        return Ok(false);
    };

    let f = tokio::fs::File::open(&path)
        .await
        .with_context(|| format!("open {}", path.display()))?;
    let mut seekable = tokio::io::BufReader::new(f);
    use tokio::io::AsyncSeekExt;
    seekable.seek(std::io::SeekFrom::Start(start_byte)).await?;
    let mut lines = seekable.lines();

    let mut batch = Vec::with_capacity(deps.batch_size);
    let mut consumed_bytes = start_byte;
    while batch.len() < deps.batch_size {
        let Some(line) = lines.next_line().await? else {
            break;
        };
        consumed_bytes += line.len() as u64 + 1;
        let env: EventEnvelope = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(e) => {
                warn!(error = %e, "skip malformed JSONL line");
                continue;
            }
        };
        if let Some(sid) = session_id_of(&env.event) {
            match deps.cache.get(sid).unwrap_or(ShareDecision::Pending) {
                ShareDecision::Allowed => batch.push(env),
                ShareDecision::Pending => {
                    break; /* hold; do not advance */
                }
                ShareDecision::DeniedKeepLocal => { /* skip-ship, advance offset */ }
            }
        } else {
            // Events without session_id — ship.
            batch.push(env);
        }
    }

    if batch.is_empty() {
        return Ok(false);
    }

    let body = serde_json::to_vec(&serde_json::json!({
        "events": batch
    }))?;
    post_batch(deps, client, &body).await?;

    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let new_off = SyncOffset {
        file: Some(filename),
        byte_offset: consumed_bytes,
    };
    new_off.save(&deps.raw_dir)?;
    info!(shipped = batch.len(), "team_sync batch posted");
    Ok(true)
}

fn session_id_of(e: &IngestEvent) -> Option<SessionId> {
    use IngestEvent::*;
    match e {
        SessionStart { session_id, .. } => Some(*session_id),
        UserPrompt { session_id, .. } => Some(*session_id),
        ToolCallEnd { session_id, .. } => *session_id,
        FileDiff { session_id, .. } => Some(*session_id),
        _ => None,
    }
}

fn select_jsonl_file(deps: &TeamSyncDeps, offset: &SyncOffset) -> Result<(Option<PathBuf>, u64)> {
    if let Some(name) = offset.file.as_deref() {
        let p = deps.raw_dir.join(name);
        if p.exists() {
            let len = std::fs::metadata(&p)?.len();
            if len > offset.byte_offset {
                return Ok((Some(p), offset.byte_offset));
            }
        }
    }
    let mut newest: Option<(PathBuf, std::time::SystemTime)> = None;
    for entry in std::fs::read_dir(&deps.raw_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let mtime = entry.metadata()?.modified()?;
        if let Some((_, prev)) = &newest {
            if mtime <= *prev {
                continue;
            }
        }
        newest = Some((path, mtime));
    }
    let next_path = newest.map(|(p, _)| p);
    let start = if let Some(p) = &next_path {
        let same_file = offset.file.as_deref().map(|n| deps.raw_dir.join(n)) == Some(p.clone());
        if same_file {
            offset.byte_offset
        } else {
            0
        }
    } else {
        0
    };
    Ok((next_path, start))
}

async fn post_batch(deps: &TeamSyncDeps, client: &reqwest::Client, body: &[u8]) -> Result<()> {
    let url = format!("{}/v1/ingest", deps.team_cfg.server_url);
    let mut attempt = 0u32;
    let mut backoff = Duration::from_secs(1);
    loop {
        attempt += 1;
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let claims = ProofClaims {
            htm: "POST".into(),
            htu: url.clone(),
            iat: now,
            jti: format!(
                "jti-{}-{}",
                Instant::now().elapsed().as_nanos(),
                uuid::Uuid::new_v4()
            ),
            ath: token_hash_hex(&deps.team_cfg.device_token),
            bsh: body_hash_hex(body),
        };
        let proof = sign(&claims, &deps.signing_key);
        let resp = client
            .post(&url)
            .header(
                "Authorization",
                format!("Bearer {}", deps.team_cfg.device_token),
            )
            .header("X-Teramind-Proof", proof)
            .header("Content-Type", "application/json")
            .body(body.to_vec())
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => return Ok(()),
            Ok(r) if r.status().is_client_error() => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                return Err(anyhow::anyhow!("ingest {status}: {text}"));
            }
            Ok(r) => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                warn!(%status, body = %text, attempt, "ingest 5xx, retrying");
            }
            Err(e) => warn!(error = %e, attempt, "ingest network error, retrying"),
        }
        if attempt >= deps.max_attempts {
            return Err(anyhow::anyhow!("ingest exhausted retries"));
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}
