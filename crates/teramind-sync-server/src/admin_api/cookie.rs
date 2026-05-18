//! Self-validating session cookie. Format:
//!   token = base64url(payload) || "." || base64url(hmac_sha256(secret, payload))
//!   payload = jti(16 bytes) || expires_at_unix_be64(8 bytes)

use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use rand::RngExt;
use sha2::Sha256;
use thiserror::Error;
use time::OffsetDateTime;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Error, PartialEq)]
pub enum CookieError {
    #[error("malformed token: not two dot-separated parts")]
    Malformed,
    #[error("base64 decode failed")]
    BadBase64,
    #[error("payload length not 24 bytes")]
    BadPayloadLength,
    #[error("HMAC verification failed")]
    BadHmac,
    #[error("expired")]
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminSession {
    pub jti: [u8; 16],
    pub expires_at: OffsetDateTime,
}

/// Generate a random 16-byte jti from the OS RNG.
pub fn random_jti() -> [u8; 16] {
    let mut out = [0u8; 16];
    rand::rng().fill(&mut out[..]);
    out
}

pub fn encode(session: &AdminSession, secret_hex: &str) -> String {
    let secret = hex::decode(secret_hex).expect("admin_session_secret must be hex");
    let mut payload = Vec::with_capacity(24);
    payload.extend_from_slice(&session.jti);
    payload.extend_from_slice(&session.expires_at.unix_timestamp().to_be_bytes());

    let mut mac = HmacSha256::new_from_slice(&secret).expect("hmac key");
    mac.update(&payload);
    let sig = mac.finalize().into_bytes();

    let e = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    format!("{}.{}", e.encode(&payload), e.encode(sig))
}

pub fn decode(
    token: &str,
    secret_hex: &str,
    now: OffsetDateTime,
) -> Result<AdminSession, CookieError> {
    let secret = hex::decode(secret_hex).map_err(|_| CookieError::BadBase64)?;
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 2 {
        return Err(CookieError::Malformed);
    }
    let e = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let payload = e.decode(parts[0]).map_err(|_| CookieError::BadBase64)?;
    let sig = e.decode(parts[1]).map_err(|_| CookieError::BadBase64)?;
    if payload.len() != 24 {
        return Err(CookieError::BadPayloadLength);
    }

    let mut mac = HmacSha256::new_from_slice(&secret).map_err(|_| CookieError::BadHmac)?;
    mac.update(&payload);
    mac.verify_slice(&sig).map_err(|_| CookieError::BadHmac)?;

    let mut jti = [0u8; 16];
    jti.copy_from_slice(&payload[..16]);
    let mut ts_bytes = [0u8; 8];
    ts_bytes.copy_from_slice(&payload[16..]);
    let expires_at = OffsetDateTime::from_unix_timestamp(i64::from_be_bytes(ts_bytes))
        .map_err(|_| CookieError::BadPayloadLength)?;
    if expires_at < now {
        return Err(CookieError::Expired);
    }
    Ok(AdminSession { jti, expires_at })
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::Duration;

    fn fixture() -> (String, AdminSession, OffsetDateTime) {
        let secret = hex::encode([0xABu8; 32]);
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let session = AdminSession {
            jti: [7u8; 16],
            expires_at: now + Duration::hours(12),
        };
        (secret, session, now)
    }

    #[test]
    fn encode_decode_roundtrips() {
        let (s, sess, now) = fixture();
        let token = encode(&sess, &s);
        let out = decode(&token, &s, now).unwrap();
        assert_eq!(out, sess);
    }

    #[test]
    fn expired_token_rejects() {
        let (s, sess, now) = fixture();
        let token = encode(&sess, &s);
        let later = now + Duration::hours(13);
        assert_eq!(decode(&token, &s, later), Err(CookieError::Expired));
    }

    #[test]
    fn tampered_hmac_rejects() {
        let (s, sess, now) = fixture();
        let mut token = encode(&sess, &s);
        let last = token.pop().unwrap();
        token.push(if last == 'A' { 'B' } else { 'A' });
        assert_eq!(decode(&token, &s, now), Err(CookieError::BadHmac));
    }

    #[test]
    fn wrong_secret_rejects() {
        let (s, sess, now) = fixture();
        let token = encode(&sess, &s);
        let other = hex::encode([0x11u8; 32]);
        assert_eq!(decode(&token, &other, now), Err(CookieError::BadHmac));
    }

    #[test]
    fn malformed_token_rejects() {
        let (s, _sess, now) = fixture();
        assert_eq!(
            decode("only-one-part", &s, now),
            Err(CookieError::Malformed)
        );
    }
}
