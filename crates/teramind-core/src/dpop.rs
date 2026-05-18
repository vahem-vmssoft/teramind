//! Shared DPoP types (RFC 9449 with `ath` + `bsh` additions).
//! Sign + hash helpers are here so both the central server and remote
//! daemons can use them. The server's verify + replay cache stay in
//! `teramind-sync-server::proof`.

use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofClaims {
    pub htm: String,
    pub htu: String,
    pub iat: i64,
    pub jti: String,
    pub ath: String,
    pub bsh: String,
}

pub fn body_hash_hex(body: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(body);
    hex::encode(h.finalize())
}

pub fn token_hash_hex(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    hex::encode(h.finalize())
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn sign(claims: &ProofClaims, signing_key: &SigningKey) -> String {
    let header = br#"{"alg":"EdDSA","typ":"dpop+jwt"}"#;
    let claims_json = serde_json::to_vec(claims).expect("claims serialize");
    let signing_input = format!("{}.{}", b64url(header), b64url(&claims_json));
    let sig: Signature = signing_key.sign(signing_input.as_bytes());
    format!("{signing_input}.{}", b64url(&sig.to_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::RngExt;

    #[test]
    fn sign_produces_three_segments() {
        let mut seed = [0u8; 32];
        rand::rng().fill(&mut seed[..]);
        let sk = SigningKey::from_bytes(&seed);
        let claims = ProofClaims {
            htm: "POST".into(),
            htu: "https://srv/v1/ingest".into(),
            iat: 1_700_000_000,
            jti: "test".into(),
            ath: token_hash_hex("tmd_v1_X"),
            bsh: body_hash_hex(b""),
        };
        let header = sign(&claims, &sk);
        assert_eq!(header.split('.').count(), 3);
    }
}
