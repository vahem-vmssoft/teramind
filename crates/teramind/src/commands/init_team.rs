//! `teramind init --team --server=… --invite=…` body.

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use ed25519_dalek::SigningKey;
use rand::{rngs::OsRng, RngCore};
use serde::Deserialize;
use teramind_core::team::{default_config_dir, save_signing_key, TeamConfig};

#[derive(Deserialize)]
struct RedeemResponse {
    user_id: String,
    device_id: String,
    device_token: String,
    device_name: String,
}

pub async fn run(server: String, invite: String, device_name: Option<String>) -> Result<()> {
    let cfg_dir = default_config_dir();
    let team_toml = cfg_dir.join("team.toml");
    if team_toml.exists() {
        return Err(anyhow!(
            "team mode already configured at {}; remove it first to re-init",
            team_toml.display()
        ));
    }

    let server = server.trim_end_matches('/').to_string();
    let device_name = device_name
        .or_else(|| hostname::get().ok().and_then(|s| s.into_string().ok()))
        .unwrap_or_else(|| "unknown-device".into());

    // Generate Ed25519 keypair.
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let sk = SigningKey::from_bytes(&seed);
    let pk = sk.verifying_key().to_bytes();
    let pk_b64 = base64::engine::general_purpose::STANDARD.encode(pk);

    // POST /v1/auth/redeem.
    let body = serde_json::json!({
        "invite_code": invite,
        "device_name": device_name,
        "device_public_key_b64": pk_b64,
    });
    let url = format!("{server}/v1/auth/redeem");
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("redeem failed: HTTP {} — {}", status, text));
    }
    let r: RedeemResponse =
        serde_json::from_str(&text).with_context(|| format!("parse redeem response: {text}"))?;

    let cfg = TeamConfig {
        server_url: server.clone(),
        user_email: "(set by server)".into(),
        user_id: r.user_id,
        device_id: r.device_id,
        device_token: r.device_token,
        device_name: r.device_name,
        redeemed_at: time::OffsetDateTime::now_utc(),
    };
    cfg.save(&team_toml)?;
    save_signing_key(&cfg_dir.join("team-key"), &sk)?;

    println!("team mode configured:");
    println!("  server:  {}", cfg.server_url);
    println!("  device:  {} ({})", cfg.device_name, cfg.device_id);
    println!("  user_id: {}", cfg.user_id);
    println!("  config:  {}", team_toml.display());
    println!(
        "  key:     {} (mode 0600)",
        cfg_dir.join("team-key").display()
    );
    println!();
    println!("Start the daemon with `teramind start` to begin shipping captures.");
    Ok(())
}
