//! Null codify provider — for tests and the `provider = "null"` opt-out path.

use async_trait::async_trait;
use teramind_core::codify::{CodifyDecision, CodifyProvider, CodifyRequest, CodifyResult};

pub struct NullCodifyProvider;

#[async_trait]
impl CodifyProvider for NullCodifyProvider {
    async fn codify(&self, _req: CodifyRequest) -> anyhow::Result<CodifyResult> {
        Ok(CodifyResult {
            decision: CodifyDecision::Skip { reason: "null provider".into() },
            input_tokens: 0,
            output_tokens: 0,
        })
    }
    fn name(&self) -> &str { "null" }
}
