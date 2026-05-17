//! When team.toml exists AND no marker exists in cwd, the SessionStart hook
//! must emit the share-prompt notice.

use ed25519_dalek::SigningKey;
use rand::{rngs::OsRng, RngCore};
use teramind_core::team::{save_signing_key, TeamConfig};

#[test]
fn session_start_with_team_mode_and_no_marker_emits_prompt() {
    let cfg_dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", cfg_dir.path());

    // Make team-mode look configured.
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let sk = SigningKey::from_bytes(&seed);
    let team_dir = cfg_dir.path().join("teramind");
    std::fs::create_dir_all(&team_dir).unwrap();
    let cfg = TeamConfig {
        server_url: "https://srv".into(),
        user_email: "alice@acme.dev".into(),
        user_id: uuid::Uuid::new_v4().to_string(),
        device_id: uuid::Uuid::new_v4().to_string(),
        device_token: "tmd_v1_X".into(),
        device_name: "x".into(),
        redeemed_at: time::OffsetDateTime::now_utc(),
    };
    cfg.save(&team_dir.join("team.toml")).unwrap();
    save_signing_key(&team_dir.join("team-key"), &sk).unwrap();

    let proj = tempfile::tempdir().unwrap();
    let result = teramind_hook::team_share_prompt::maybe_share_prompt(proj.path());
    assert!(
        result.is_some(),
        "share prompt must appear when team is configured + no marker"
    );
    let s = result.unwrap();
    assert!(
        s.contains("Share captures from this project"),
        "share prompt copy must match expected text; got: {s}"
    );
}

#[test]
fn no_prompt_when_team_not_configured() {
    let cfg_dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", cfg_dir.path());
    // No team.toml.
    let proj = tempfile::tempdir().unwrap();
    assert!(teramind_hook::team_share_prompt::maybe_share_prompt(proj.path()).is_none());
}

#[test]
fn no_prompt_when_marker_already_set() {
    let cfg_dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", cfg_dir.path());

    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let sk = SigningKey::from_bytes(&seed);
    let team_dir = cfg_dir.path().join("teramind");
    std::fs::create_dir_all(&team_dir).unwrap();
    let cfg = TeamConfig {
        server_url: "https://srv".into(),
        user_email: "x".into(),
        user_id: uuid::Uuid::new_v4().to_string(),
        device_id: uuid::Uuid::new_v4().to_string(),
        device_token: "tmd_v1_X".into(),
        device_name: "x".into(),
        redeemed_at: time::OffsetDateTime::now_utc(),
    };
    cfg.save(&team_dir.join("team.toml")).unwrap();
    save_signing_key(&team_dir.join("team-key"), &sk).unwrap();

    let proj = tempfile::tempdir().unwrap();
    let marker = teramind_core::team_share::ShareMarker {
        share: true,
        set_by: "alice".into(),
        set_at: time::OffsetDateTime::now_utc(),
    };
    teramind_core::team_share::write_marker_at_cwd(proj.path(), &marker).unwrap();
    assert!(teramind_hook::team_share_prompt::maybe_share_prompt(proj.path()).is_none());
}
