pub mod llm_proposal;
pub mod problem_fix;
pub mod tool_chain;

use crate::services::decision_cache::{DecisionCache, ShareDecision};
use std::sync::Arc;

/// Returns true when the session has `DeniedKeepLocal` in the cache.
pub(super) fn is_denied(cache: &Option<Arc<DecisionCache>>, sid: uuid::Uuid) -> bool {
    cache
        .as_deref()
        .and_then(|c| c.get(teramind_core::ids::SessionId(sid)))
        .map(|d| d == ShareDecision::DeniedKeepLocal)
        .unwrap_or(false)
}
