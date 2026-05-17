//! Shared RPC dispatch logic used by both the local daemon's IPC server and
//! the central sync server's POST /v1/rpc handler.

use crate::services::search::BlendWeights;
use std::path::PathBuf;
use std::sync::Arc;
use teramind_core::embed::EmbeddingProvider;
use teramind_core::summarize::SummaryProvider;
use teramind_db::pool::DbPool;
use teramind_db::repos::{SearchRepo, SkillCandidateRepo, SkillObservationRepo, SkillRepo, WikiRepo};
use teramind_ipc::proto::{Request, Response};

#[derive(Clone)]
pub struct RpcDeps {
    pub pool: DbPool,
    pub search_repo: SearchRepo,
    pub wiki_repo: WikiRepo,
    pub embed_provider: Arc<dyn EmbeddingProvider>,
    pub embed_model: String,
    pub search_weights: BlendWeights,
    pub summary_provider: Arc<dyn SummaryProvider>,
    pub summary_model: String,
    pub jsonl_dir: PathBuf,
    pub event_bus: Option<tokio::sync::broadcast::Sender<teramind_core::team_event::TeamEvent>>,
    pub skill_obs: SkillObservationRepo,
    pub skill_cand: SkillCandidateRepo,
    pub skill_repo: SkillRepo,
    pub min_observation_frequency: i32,
}

/// Identity of the caller — `Some` on the server-side `/v1/rpc` after auth,
/// `None` for local daemon IPC (single-user mode).
#[derive(Debug, Clone, Copy)]
pub struct AuthContext {
    pub user_id: uuid::Uuid,
    pub device_id: uuid::Uuid,
}

/// The dispatch body for read + skill-save requests.
///
/// `Status`/`Ping`/`Shutdown` (daemon control) and `TeamShareSet` (local file
/// IO) are NOT handled here — they remain in `DaemonIpcHandler::handle_request`.
pub async fn dispatch(deps: &RpcDeps, req: Request, auth: Option<AuthContext>) -> Response {
    match req {
        Request::Search(r) => {
            let out = crate::services::search::do_search_with_fallback(
                &deps.search_repo,
                &deps.jsonl_dir,
                Some(deps.embed_provider.clone()),
                &deps.embed_model,
                deps.search_weights,
                &r,
            )
            .await;
            Response::SearchResults(teramind_core::types::SearchResults {
                hits: out.hits,
                degraded: out.degraded,
                took_ms: out.took_ms,
            })
        }
        Request::Recall(r) => {
            match crate::services::search::do_recall(&deps.search_repo, &r).await {
                Ok(out) => Response::SearchResults(teramind_core::types::SearchResults {
                    hits: out.hits,
                    degraded: out.degraded,
                    took_ms: out.took_ms,
                }),
                Err(e) => Response::Error(format!("recall failed: {e}")),
            }
        }
        Request::AutoRecall(r) => {
            match crate::services::search::do_auto_recall(&deps.search_repo, &deps.wiki_repo, &deps.skill_repo, &r)
                .await
            {
                Ok(md) => Response::AutoRecallDigest {
                    markdown: md,
                    degraded: false,
                },
                Err(_) => Response::AutoRecallDigest {
                    markdown: String::new(),
                    degraded: true,
                },
            }
        }
        Request::SaveSkill(r) => match deps.search_repo.upsert_skill(&r).await {
            Ok(s) => {
                if let Some(bus) = deps.event_bus.as_ref() {
                    let user_id = auth.map(|a| a.user_id).unwrap_or_default();
                    let _ = bus.send(teramind_core::team_event::TeamEvent::SkillSaved {
                        skill_id: s.id.0,
                        user_id,
                        name: s.name.clone(),
                        ts: time::OffsetDateTime::now_utc(),
                    });
                }
                Response::SkillRef(s)
            }
            Err(e) => Response::Error(format!("save_skill failed: {e}")),
        },
        Request::WikiLookup { session_id, cwd } => {
            let result: anyhow::Result<Option<teramind_db::repos::WikiPage>> = async {
                if let Some(sid_str) = session_id {
                    let sid = teramind_core::ids::SessionId(uuid::Uuid::parse_str(&sid_str)?);
                    let p = deps
                        .wiki_repo
                        .get_for_session(sid, &deps.summary_model)
                        .await?;
                    Ok(p)
                } else if let Some(cwd) = cwd {
                    let p = deps.wiki_repo.latest_for_cwd(&cwd).await?;
                    Ok(p)
                } else {
                    Ok(None)
                }
            }
            .await;
            match result {
                Ok(Some(p)) => {
                    let session_cwd: String =
                        sqlx::query_scalar("SELECT cwd FROM sessions WHERE id = $1")
                            .bind(p.session_id.0)
                            .fetch_one(deps.pool.pg())
                            .await
                            .unwrap_or_default();
                    Response::WikiPage {
                        session_id: p.session_id.0.to_string(),
                        cwd: session_cwd,
                        model: p.model,
                        content: p.content,
                        generated_at: p.generated_at,
                    }
                }
                Ok(None) => Response::WikiNotFound,
                Err(e) => Response::Error(format!("wiki lookup failed: {e}")),
            }
        }
        Request::CodifyNow { seed_session_ids, hint } => {
            use sha2::{Digest, Sha256};
            let sids: Vec<teramind_core::ids::SessionId> = seed_session_ids.iter()
                .filter_map(|s| uuid::Uuid::parse_str(s).ok())
                .map(teramind_core::ids::SessionId)
                .collect();
            let hint_str = hint.unwrap_or_default();
            let mut h = Sha256::new();
            h.update(hint_str.as_bytes());
            h.update(format!("{:?}", sids).as_bytes());
            let sig = hex::encode(&h.finalize()[..8]);
            let ctx = serde_json::json!({ "hint": hint_str, "source": "mcp" });
            let _ = deps.skill_obs.upsert("llm_proposal", &sig,
                if sids.is_empty() { &[] } else { &sids[..] }, ctx).await;
            let obs = deps.skill_obs.find_by_sig("llm_proposal", &sig).await.ok().flatten();
            let id = obs.map(|o| o.id.0.to_string()).unwrap_or_default();
            Response::CodifyQueued { observation_id: id }
        }

        Request::SkillsList { filter, limit } => {
            let mut rows: Vec<teramind_ipc::proto::SkillRow> = vec![];
            let f = filter.unwrap_or_else(|| "all".into());
            if f == "pending" || f == "rejected" || f == "approved" {
                let cands = deps.skill_cand.list_filter(Some(&f), limit as i64).await.unwrap_or_default();
                for c in cands {
                    rows.push(teramind_ipc::proto::SkillRow {
                        id: c.id.0.to_string(),
                        name: c.name,
                        description: c.description,
                        source: "candidate".into(),
                        status: Some(c.status),
                        applies_to_cwds: c.applies_to_cwds,
                    });
                }
            } else {
                // Live skills.
                let live: Vec<(uuid::Uuid, String, String, String, Vec<String>)> = sqlx::query_as(
                    r#"SELECT id, name, description, source, applies_to_cwds
                       FROM skills ORDER BY updated_at DESC LIMIT $1"#)
                    .bind(limit as i64).fetch_all(deps.pool.pg()).await.unwrap_or_default();
                for (id, n, d, s, cwds) in live {
                    if f == "codified" && s != "codified" { continue; }
                    if f == "authored" && s != "authored" { continue; }
                    rows.push(teramind_ipc::proto::SkillRow {
                        id: id.to_string(),
                        name: n,
                        description: d,
                        source: s,
                        status: None,
                        applies_to_cwds: cwds,
                    });
                }
            }
            Response::SkillsList { rows }
        }

        Request::SkillsShow { name_or_id } => {
            let row: Option<(uuid::Uuid, String, String, String, String, Vec<String>)> = sqlx::query_as(
                r#"SELECT id, name, description, body, source, applies_to_cwds
                   FROM skills
                   WHERE name = $1 OR id::text = $1"#)
                .bind(&name_or_id).fetch_optional(deps.pool.pg()).await.unwrap_or(None);
            if let Some((_, name, description, body, source, applies_to_cwds)) = row {
                Response::SkillShow { name, description, body, source, applies_to_cwds }
            } else {
                Response::Error(format!("no skill named or with id '{name_or_id}'"))
            }
        }

        Request::SkillsObservations { kind, min_freq, status, limit } => {
            let _ = min_freq; // status takes priority; could combine
            let obs = deps.skill_obs.list_recent(kind.as_deref(), status.as_deref(), limit as i64)
                .await.unwrap_or_default();
            let rows = obs.into_iter().map(|o| teramind_ipc::proto::ObservationRow {
                id: o.id.0.to_string(),
                kind: o.kind,
                signature: o.signature,
                frequency: o.frequency,
                status: o.status,
                last_seen_at: o.last_seen_at.to_string(),
            }).collect();
            Response::SkillsObservations { rows }
        }

        // Daemon-control + local-only — not handled here.
        Request::Status | Request::Ping | Request::Shutdown | Request::TeamShareSet { .. } => {
            Response::Error("unsupported in shared dispatch".into())
        }
    }
}
