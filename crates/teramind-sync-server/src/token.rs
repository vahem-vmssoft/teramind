//! Long-lived device bearer tokens.

use rand::Rng;
use sha2::{Digest, Sha256};
use thiserror::Error;

const PREFIX: &str = "tmd_v1_";
const RAW_BYTES: usize = 32;

#[derive(Debug, Error)]
pub enum TokenError {
    #[error("token must start with tmd_v1_")]
    BadPrefix,
    #[error("token has wrong length")]
    BadLength,
    #[error("token contains an invalid character")]
    BadAlphabet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceToken {
    canonical: String,
}

impl DeviceToken {
    pub fn generate<R: Rng>(rng: &mut R) -> Self {
        let mut bytes = [0u8; RAW_BYTES];
        rng.fill_bytes(&mut bytes);
        Self::from_bytes(bytes)
    }

    pub fn from_bytes(bytes: [u8; RAW_BYTES]) -> Self {
        let body = base32::encode(base32::Alphabet::Crockford, &bytes);
        Self {
            canonical: format!("{PREFIX}{body}"),
        }
    }

    pub fn parse(input: &str) -> Result<Self, TokenError> {
        let input = input.trim();
        if !input.starts_with(PREFIX) {
            return Err(TokenError::BadPrefix);
        }
        let body = &input[PREFIX.len()..];
        // base32 of 32 bytes = ceil(32*8/5) = 52 chars.
        if body.len() != 52 {
            return Err(TokenError::BadLength);
        }
        let bytes =
            base32::decode(base32::Alphabet::Crockford, body).ok_or(TokenError::BadAlphabet)?;
        if bytes.len() != RAW_BYTES {
            return Err(TokenError::BadLength);
        }
        let mut arr = [0u8; RAW_BYTES];
        arr.copy_from_slice(&bytes);
        Ok(Self::from_bytes(arr))
    }

    pub fn as_str(&self) -> &str {
        &self.canonical
    }

    /// sha256 of the canonical wire form.
    pub fn hash(&self) -> Vec<u8> {
        let mut h = Sha256::new();
        h.update(self.canonical.as_bytes());
        h.finalize().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn roundtrips() {
        let mut rng = rand_chacha::ChaCha20Rng::seed_from_u64(1234);
        let t = DeviceToken::generate(&mut rng);
        assert_eq!(DeviceToken::parse(t.as_str()).unwrap(), t);
        assert!(t.as_str().starts_with("tmd_v1_"));
    }

    #[test]
    fn hash_is_stable_and_distinct() {
        let a = DeviceToken::from_bytes([0x10u8; 32]);
        let b = DeviceToken::from_bytes([0x11u8; 32]);
        assert_eq!(a.hash(), a.hash());
        assert_ne!(a.hash(), b.hash());
        assert_eq!(a.hash().len(), 32);
    }

    #[test]
    fn bad_prefix_errors() {
        assert!(matches!(
            DeviceToken::parse("xxx_v1_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
            Err(TokenError::BadPrefix)
        ));
    }
}
