//! Team-mode config: ~/.config/teramind/team.toml + team-key (Ed25519 32 bytes).

use anyhow::{anyhow, Context, Result};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamConfig {
    pub server_url: String,
    pub user_email: String,
    pub user_id: String,
    pub device_id: String,
    pub device_token: String,
    pub device_name: String,
    #[serde(with = "time::serde::rfc3339")]
    pub redeemed_at: time::OffsetDateTime,
}

impl TeamConfig {
    pub fn load(path: &Path) -> Result<Self> {
        ensure_secure_perms(path).context("team.toml perms")?;
        let raw = std::fs::read_to_string(path).context("read team.toml")?;
        let cfg: TeamConfig = toml::from_str(&raw).context("parse team.toml")?;
        Ok(cfg)
    }

    /// Writes team.toml with mode 0600. Overwrites if present.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("create config dir")?;
        }
        let raw = toml::to_string(self).context("serialize team.toml")?;
        write_secure(path, raw.as_bytes())
    }
}

/// Read the 32-byte raw Ed25519 private key from `team-key`. Enforces 0600.
pub fn load_signing_key(path: &Path) -> Result<SigningKey> {
    ensure_secure_perms(path).context("team-key perms")?;
    let bytes = std::fs::read(path).context("read team-key")?;
    if bytes.len() != 32 {
        return Err(anyhow!(
            "team-key must be exactly 32 bytes (got {})",
            bytes.len()
        ));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(SigningKey::from_bytes(&arr))
}

/// Write a 32-byte Ed25519 private key to disk with mode 0600.
pub fn save_signing_key(path: &Path, key: &SigningKey) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create config dir")?;
    }
    write_secure(path, &key.to_bytes())
}

/// Default config directory: $XDG_CONFIG_HOME/teramind or ~/.config/teramind.
pub fn default_config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg).join("teramind")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config").join("teramind")
    } else {
        PathBuf::from(".").join(".teramind")
    }
}

fn write_secure(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("open {}", path.display()))?;
        f.write_all(bytes)?;
    }
    #[cfg(not(unix))]
    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .with_context(|| format!("open {}", path.display()))?;
        f.write_all(bytes)?;
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_secure_perms(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let md = std::fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    let mode = md.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(anyhow!(
            "{} has insecure perms {:#o}; chmod 0600 to fix",
            path.display(),
            mode
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_secure_perms(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::{rngs::OsRng, RngCore};

    fn random_key() -> SigningKey {
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        SigningKey::from_bytes(&seed)
    }

    #[test]
    fn team_config_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("team.toml");
        let cfg = TeamConfig {
            server_url: "https://srv".into(),
            user_email: "alice@acme.dev".into(),
            user_id: uuid::Uuid::new_v4().to_string(),
            device_id: uuid::Uuid::new_v4().to_string(),
            device_token: "tmd_v1_XYZ".into(),
            device_name: "alice-mac".into(),
            redeemed_at: time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        };
        cfg.save(&path).unwrap();
        let loaded = TeamConfig::load(&path).unwrap();
        assert_eq!(loaded.device_token, "tmd_v1_XYZ");
        assert_eq!(loaded.server_url, "https://srv");
    }

    #[test]
    fn signing_key_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("team-key");
        let original = random_key();
        save_signing_key(&path, &original).unwrap();
        let loaded = load_signing_key(&path).unwrap();
        assert_eq!(original.to_bytes(), loaded.to_bytes());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_insecure_perms_on_load() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("team.toml");
        std::fs::write(&path, "bogus").unwrap();
        let mut p = std::fs::metadata(&path).unwrap().permissions();
        p.set_mode(0o644);
        std::fs::set_permissions(&path, p).unwrap();
        assert!(TeamConfig::load(&path).is_err(), "0644 must be rejected");
    }
}
