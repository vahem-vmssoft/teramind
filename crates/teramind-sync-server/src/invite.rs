//! Invite-code generation / parsing / hashing.

use rand::RngCore;
use sha2::{Digest, Sha256};
use thiserror::Error;

const PREFIX: &str = "TM";
const RAW_BYTES: usize = 16;

#[derive(Debug, Error)]
pub enum InviteError {
    #[error("invite code must start with TM-")]
    BadPrefix,
    #[error("invite code has wrong length")]
    BadLength,
    #[error("invite code contains an invalid character")]
    BadAlphabet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteCode {
    canonical: String,
}

impl InviteCode {
    pub fn generate<R: RngCore>(rng: &mut R) -> Self {
        let mut bytes = [0u8; RAW_BYTES];
        rng.fill_bytes(&mut bytes);
        Self::from_bytes(bytes)
    }

    pub fn from_bytes(bytes: [u8; RAW_BYTES]) -> Self {
        let alphabet = base32::Alphabet::Crockford;
        let body = base32::encode(alphabet, &bytes);
        // Group into 4-char chunks for legibility.
        let mut chunks: Vec<String> = body
            .as_bytes()
            .chunks(4)
            .map(|c| String::from_utf8_lossy(c).into_owned())
            .collect();
        chunks.insert(0, PREFIX.into());
        Self {
            canonical: chunks.join("-"),
        }
    }

    pub fn parse(input: &str) -> Result<Self, InviteError> {
        let cleaned: String = input
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .map(|c| c.to_ascii_uppercase())
            .collect();
        if !cleaned.starts_with(PREFIX) {
            return Err(InviteError::BadPrefix);
        }
        let body = &cleaned[PREFIX.len()..];
        // Crockford base32 of 16 bytes = ceil(16*8/5) = 26 chars.
        if body.len() != 26 {
            return Err(InviteError::BadLength);
        }
        let bytes =
            base32::decode(base32::Alphabet::Crockford, body).ok_or(InviteError::BadAlphabet)?;
        let mut arr = [0u8; RAW_BYTES];
        if bytes.len() != RAW_BYTES {
            return Err(InviteError::BadLength);
        }
        arr.copy_from_slice(&bytes);
        Ok(Self::from_bytes(arr))
    }

    pub fn as_str(&self) -> &str {
        &self.canonical
    }

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
    fn generate_then_parse_roundtrips() {
        let mut rng = rand_chacha::ChaCha20Rng::seed_from_u64(0xC0FFEE);
        let c = InviteCode::generate(&mut rng);
        let parsed = InviteCode::parse(c.as_str()).unwrap();
        assert_eq!(c, parsed);
        assert!(c.as_str().starts_with("TM-"));
    }

    #[test]
    fn parse_is_case_insensitive_and_whitespace_tolerant() {
        let c = InviteCode::from_bytes([0x42u8; 16]);
        let lower = c.as_str().to_lowercase();
        let spaced = format!(" {lower} ");
        assert_eq!(InviteCode::parse(&spaced).unwrap(), c);
    }

    #[test]
    fn bad_prefix_errors() {
        assert!(matches!(
            InviteCode::parse("XX-1234-5678-9ABC-DEFG-HJKM-NPQR-STVW"),
            Err(InviteError::BadPrefix)
        ));
    }

    #[test]
    fn bad_length_errors() {
        assert!(matches!(
            InviteCode::parse("TM-1234"),
            Err(InviteError::BadLength)
        ));
    }

    #[test]
    fn bad_alphabet_errors() {
        // '@' is not in any base32 alphabet — guaranteed to fail decoding.
        let bad = "TM-@@@@-@@@@-@@@@-@@@@-@@@@-@@@@-@@";
        assert!(matches!(
            InviteCode::parse(bad),
            Err(InviteError::BadAlphabet)
        ));
    }

    #[test]
    fn hash_is_stable() {
        let c = InviteCode::from_bytes([0x42u8; 16]);
        assert_eq!(c.hash(), c.hash());
        let c2 = InviteCode::from_bytes([0x43u8; 16]);
        assert_ne!(c.hash(), c2.hash());
    }
}
