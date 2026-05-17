//! DPoP-signed HTTPS transport. POSTs to {server}/v1/rpc.

use async_trait::async_trait;
use ed25519_dalek::SigningKey;
use std::sync::Arc;
use teramind_core::dpop::{body_hash_hex, sign, token_hash_hex, ProofClaims};
use teramind_core::team::TeamConfig;
use teramind_ipc::proto::{Request, Response};
use teramind_ipc::rpc_transport::{RpcError, RpcTransport};
use time::OffsetDateTime;

pub struct HttpsTransport {
    cfg: Arc<TeamConfig>,
    key: Arc<SigningKey>,
    http: reqwest::Client,
}

impl HttpsTransport {
    pub fn new(cfg: Arc<TeamConfig>, key: Arc<SigningKey>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("reqwest client");
        Self { cfg, key, http }
    }
}

#[async_trait]
impl RpcTransport for HttpsTransport {
    async fn request(&self, req: Request) -> Result<Response, RpcError> {
        let url = format!("{}/v1/rpc", self.cfg.server_url);
        let body = serde_json::to_vec(&req).map_err(|e| RpcError::Other(e.to_string()))?;
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let claims = ProofClaims {
            htm: "POST".into(),
            htu: url.clone(),
            iat: now,
            jti: format!("jti-{}", uuid::Uuid::new_v4()),
            ath: token_hash_hex(&self.cfg.device_token),
            bsh: body_hash_hex(&body),
        };
        let proof = sign(&claims, &self.key);
        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.cfg.device_token))
            .header("X-Teramind-Proof", proof)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| RpcError::Connect(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(RpcError::Server(format!(
                "{}: {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            )));
        }
        resp.json::<Response>()
            .await
            .map_err(|e| RpcError::Decode(e.to_string()))
    }
}
