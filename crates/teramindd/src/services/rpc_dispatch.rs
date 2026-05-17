//! Shared RPC dispatch logic used by both the local daemon's IPC server and
//! the central sync server's POST /v1/rpc handler.

use crate::services::search::BlendWeights;
use std::path::PathBuf;
use std::sync::Arc;
use teramind_core::embed::EmbeddingProvider;
use teramind_core::summarize::SummaryProvider;
use teramind_db::pool::DbPool;
use teramind_db::repos::{SearchRepo, WikiRepo};
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
pub async fn dispatch(deps: &RpcDeps, req: Request, _auth: Option<AuthContext>) -> Response {
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
            match crate::services::search::do_auto_recall(&deps.search_repo, &deps.wiki_repo, &r)
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
            Ok(s) => Response::SkillRef(s),
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
        // Daemon-control + local-only — not handled here.
        Request::Status | Request::Ping | Request::Shutdown | Request::TeamShareSet { .. } => {
            Response::Error("unsupported in shared dispatch".into())
        }
    }
}
