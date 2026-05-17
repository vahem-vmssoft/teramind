//! DPoP-style request signing (Ed25519). RFC 9449 with our additions
//! (`ath`, `bsh`).

use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofClaims {
    pub htm: String,
    pub htu: String,
    pub iat: i64,
    pub jti: String,
    pub ath: String, // hex(sha256(bearer_token))
    pub bsh: String, // hex(sha256(request_body))
}

#[derive(Debug, Error, PartialEq)]
pub enum ProofError {
    #[error("header is not a valid 3-part JWS compact form")]
    Malformed,
    #[error("base64url decoding failed")]
    BadBase64,
    #[error("JSON claims failed to parse")]
    BadClaims,
    #[error("signature verification failed")]
    BadSignature,
    #[error("iat outside ±{0}s of now")]
    StaleIat(i64),
    #[error("htm does not match request")]
    HtmMismatch,
    #[error("htu does not match request")]
    HtuMismatch,
    #[error("ath does not match bearer token")]
    AthMismatch,
    #[error("bsh does not match body")]
    BshMismatch,
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

fn b64url_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn b64url_decode(s: &str) -> Result<Vec<u8>, ProofError> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|_| ProofError::BadBase64)
}

pub fn sign(claims: &ProofClaims, signing_key: &SigningKey) -> String {
    // Compact JWS: base64url(header).base64url(claims).base64url(signature)
    let header = br#"{"alg":"EdDSA","typ":"dpop+jwt"}"#;
    let claims_json = serde_json::to_vec(claims).expect("claims serialize");
    let h_b64 = b64url_encode(header);
    let c_b64 = b64url_encode(&claims_json);
    let signing_input = format!("{h_b64}.{c_b64}");
    let sig: Signature = signing_key.sign(signing_input.as_bytes());
    let s_b64 = b64url_encode(&sig.to_bytes());
    format!("{signing_input}.{s_b64}")
}

#[allow(clippy::too_many_arguments)]
pub fn verify(
    header: &str,
    public_key_bytes: &[u8],
    expected_method: &str,
    expected_url: &str,
    expected_body_hash_hex: &str,
    expected_token_hash_hex: &str,
    now_unix: i64,
    skew_secs: i64,
) -> Result<ProofClaims, ProofError> {
    let parts: Vec<&str> = header.split('.').collect();
    if parts.len() != 3 {
        return Err(ProofError::Malformed);
    }
    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let sig_bytes = b64url_decode(parts[2]).map_err(|_| ProofError::BadSignature)?;
    if sig_bytes.len() != 64 {
        return Err(ProofError::BadSignature);
    }
    let mut sig_arr = [0u8; 64];
    sig_arr.copy_from_slice(&sig_bytes);
    let sig = Signature::from_bytes(&sig_arr);

    if public_key_bytes.len() != 32 {
        return Err(ProofError::BadSignature);
    }
    let mut pk_arr = [0u8; 32];
    pk_arr.copy_from_slice(public_key_bytes);
    let pk = VerifyingKey::from_bytes(&pk_arr).map_err(|_| ProofError::BadSignature)?;
    pk.verify(signing_input.as_bytes(), &sig)
        .map_err(|_| ProofError::BadSignature)?;

    let claims_bytes = b64url_decode(parts[1])?;
    let claims: ProofClaims =
        serde_json::from_slice(&claims_bytes).map_err(|_| ProofError::BadClaims)?;

    if claims.htm != expected_method {
        return Err(ProofError::HtmMismatch);
    }
    if claims.htu != expected_url {
        return Err(ProofError::HtuMismatch);
    }
    if claims.ath != expected_token_hash_hex {
        return Err(ProofError::AthMismatch);
    }
    if claims.bsh != expected_body_hash_hex {
        return Err(ProofError::BshMismatch);
    }
    if (now_unix - claims.iat).abs() > skew_secs {
        return Err(ProofError::StaleIat(skew_secs));
    }

    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::{rngs::OsRng, RngCore};

    fn fresh_keypair() -> (SigningKey, Vec<u8>) {
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        let sk = SigningKey::from_bytes(&seed);
        let pk = sk.verifying_key().to_bytes().to_vec();
        (sk, pk)
    }

    fn happy_claims(token: &str, body: &[u8], now: i64) -> ProofClaims {
        ProofClaims {
            htm: "POST".into(),
            htu: "https://srv/v1/ingest".into(),
            iat: now,
            jti: "deadbeef0123".into(),
            ath: token_hash_hex(token),
            bsh: body_hash_hex(body),
        }
    }

    #[test]
    fn sign_then_verify_happy() {
        let (sk, pk) = fresh_keypair();
        let now = 1_700_000_000;
        let token = "tmd_v1_AAAA";
        let body = br#"{"x":1}"#;
        let c = happy_claims(token, body, now);
        let header = sign(&c, &sk);
        let out = verify(
            &header,
            &pk,
            "POST",
            "https://srv/v1/ingest",
            &body_hash_hex(body),
            &token_hash_hex(token),
            now,
            60,
        )
        .unwrap();
        assert_eq!(out.jti, "deadbeef0123");
    }

    #[test]
    fn wrong_public_key_fails() {
        let (sk, _) = fresh_keypair();
        let (_, other_pk) = fresh_keypair();
        let now = 1_700_000_000;
        let body = b"";
        let c = happy_claims("tmd_v1_X", body, now);
        let header = sign(&c, &sk);
        let err = verify(
            &header,
            &other_pk,
            "POST",
            "https://srv/v1/ingest",
            &body_hash_hex(body),
            &token_hash_hex("tmd_v1_X"),
            now,
            60,
        )
        .unwrap_err();
        assert_eq!(err, ProofError::BadSignature);
    }

    #[test]
    fn wrong_method_fails() {
        let (sk, pk) = fresh_keypair();
        let now = 1_700_000_000;
        let c = happy_claims("tmd_v1_X", b"", now);
        let header = sign(&c, &sk);
        let err = verify(
            &header,
            &pk,
            "GET",
            "https://srv/v1/ingest",
            &body_hash_hex(b""),
            &token_hash_hex("tmd_v1_X"),
            now,
            60,
        )
        .unwrap_err();
        assert_eq!(err, ProofError::HtmMismatch);
    }

    #[test]
    fn wrong_url_fails() {
        let (sk, pk) = fresh_keypair();
        let now = 1_700_000_000;
        let c = happy_claims("tmd_v1_X", b"", now);
        let header = sign(&c, &sk);
        let err = verify(
            &header,
            &pk,
            "POST",
            "https://srv/v1/rpc",
            &body_hash_hex(b""),
            &token_hash_hex("tmd_v1_X"),
            now,
            60,
        )
        .unwrap_err();
        assert_eq!(err, ProofError::HtuMismatch);
    }

    #[test]
    fn tampered_body_fails() {
        let (sk, pk) = fresh_keypair();
        let now = 1_700_000_000;
        let c = happy_claims("tmd_v1_X", b"clean", now);
        let header = sign(&c, &sk);
        let err = verify(
            &header,
            &pk,
            "POST",
            "https://srv/v1/ingest",
            &body_hash_hex(b"tampered"),
            &token_hash_hex("tmd_v1_X"),
            now,
            60,
        )
        .unwrap_err();
        assert_eq!(err, ProofError::BshMismatch);
    }

    #[test]
    fn token_mismatch_fails() {
        let (sk, pk) = fresh_keypair();
        let now = 1_700_000_000;
        let c = happy_claims("tmd_v1_X", b"", now);
        let header = sign(&c, &sk);
        let err = verify(
            &header,
            &pk,
            "POST",
            "https://srv/v1/ingest",
            &body_hash_hex(b""),
            &token_hash_hex("tmd_v1_OTHER"),
            now,
            60,
        )
        .unwrap_err();
        assert_eq!(err, ProofError::AthMismatch);
    }

    #[test]
    fn stale_iat_fails() {
        let (sk, pk) = fresh_keypair();
        let signed_at = 1_700_000_000;
        let way_later = signed_at + 120;
        let c = happy_claims("tmd_v1_X", b"", signed_at);
        let header = sign(&c, &sk);
        let err = verify(
            &header,
            &pk,
            "POST",
            "https://srv/v1/ingest",
            &body_hash_hex(b""),
            &token_hash_hex("tmd_v1_X"),
            way_later,
            60,
        )
        .unwrap_err();
        assert_eq!(err, ProofError::StaleIat(60));
    }

    #[test]
    fn flipped_signature_byte_fails() {
        let (sk, pk) = fresh_keypair();
        let now = 1_700_000_000;
        let c = happy_claims("tmd_v1_X", b"", now);
        let mut header = sign(&c, &sk);
        // Flip the last char of the signature segment.
        let last = header.pop().unwrap();
        let new = if last == 'A' { 'B' } else { 'A' };
        header.push(new);
        let err = verify(
            &header,
            &pk,
            "POST",
            "https://srv/v1/ingest",
            &body_hash_hex(b""),
            &token_hash_hex("tmd_v1_X"),
            now,
            60,
        )
        .unwrap_err();
        assert_eq!(err, ProofError::BadSignature);
    }
}

pub mod replay {
    use parking_lot::Mutex;
    use std::collections::{HashMap, VecDeque};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use teramind_core::ids::DeviceId;

    pub struct ReplayCache {
        max_per_device: usize,
        ttl: Duration,
        // (jti, inserted_at)
        inner: Mutex<HashMap<DeviceId, VecDeque<(String, Instant)>>>,
    }

    impl ReplayCache {
        pub fn new(max_per_device: usize, ttl_secs: u64) -> Arc<Self> {
            Arc::new(Self {
                max_per_device,
                ttl: Duration::from_secs(ttl_secs),
                inner: Mutex::new(HashMap::new()),
            })
        }

        /// Returns true if `jti` was newly inserted; false if it's a replay.
        pub fn check_and_insert(&self, device: DeviceId, jti: &str) -> bool {
            let now = Instant::now();
            let mut map = self.inner.lock();
            let q = map.entry(device).or_default();

            // Drop expired entries from the front.
            while let Some((_, ts)) = q.front() {
                if now.duration_since(*ts) > self.ttl {
                    q.pop_front();
                } else {
                    break;
                }
            }

            if q.iter().any(|(j, _)| j == jti) {
                return false;
            }

            q.push_back((jti.to_string(), now));
            while q.len() > self.max_per_device {
                q.pop_front();
            }
            true
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use teramind_core::ids::DeviceId;
        use uuid::Uuid;

        #[test]
        fn first_insert_returns_true_replay_returns_false() {
            let c = ReplayCache::new(8, 60);
            let d = DeviceId(Uuid::new_v4());
            assert!(c.check_and_insert(d, "j1"));
            assert!(!c.check_and_insert(d, "j1"));
            assert!(c.check_and_insert(d, "j2"));
        }

        #[test]
        fn distinct_devices_are_isolated() {
            let c = ReplayCache::new(8, 60);
            let a = DeviceId(Uuid::new_v4());
            let b = DeviceId(Uuid::new_v4());
            assert!(c.check_and_insert(a, "j1"));
            assert!(c.check_and_insert(b, "j1"));
        }

        #[test]
        fn cap_evicts_oldest() {
            let c = ReplayCache::new(2, 60);
            let d = DeviceId(Uuid::new_v4());
            assert!(c.check_and_insert(d, "j1"));
            assert!(c.check_and_insert(d, "j2"));
            assert!(c.check_and_insert(d, "j3"));
            // j1 should have been evicted by capacity; a re-insert succeeds.
            assert!(c.check_and_insert(d, "j1"));
        }
    }
}
