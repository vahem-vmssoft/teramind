//! Dashboard §6 — every bus.send(TeamEvent::*) call site is paired with an
//! event_log_writer.log() so the team_event_log table reflects the same
//! stream that live WebSocket subscribers see.
//!
//! Production emission sites today:
//!   - SessionEnded   → handlers::ingest::publish_on_success (sync-server)
//!   - SkillSaved     → teramindd::services::rpc_dispatch SaveSkill arm
//!   - WikiPageReady  → reserved in TeamEvent enum; no production publisher
//!     yet, but the writer handles the variant so a row is persisted when
//!     it is wired up.
//!
//! This test invokes EventLogWriter::log directly for each variant (same
//! type the production sites pass through) and asserts a row appears in
//! team_event_log with the corresponding `kind` string.

use std::net::SocketAddr;
use teramind_core::team_event::TeamEvent;
use teramind_db::repos::UserRepo;
use teramind_sync_server::config::*;
use teramind_sync_server::state::AppState;
use uuid::Uuid;

async fn seed_user(state: &AppState, email: &str) -> Uuid {
    let users = UserRepo::new(state.pool.clone());
    users.upsert_by_email(email, None).await.unwrap().id.0
}

fn admin_cfg() -> AdminConfig {
    use argon2::password_hash::{rand_core::OsRng, SaltString};
    use argon2::{Argon2, PasswordHasher};
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(b"hunter2hunter2", &salt)
        .unwrap()
        .to_string();
    AdminConfig {
        admin_password_hash: hash,
        admin_session_secret: "ab".repeat(32),
        admin_session_ttl_hours: 12,
        event_log_retention_days: 90,
    }
}

async fn boot() -> anyhow::Result<(SocketAddr, AppState)> {
    let pool = teramind_db::testing::fresh_pool().await?;
    let cfg = ServerConfig {
        listen_addr: "127.0.0.1:0".into(),
        database_url: "ignored".into(),
        tls: None,
        auth: AuthConfig::default(),
        ingest: IngestConfig::default(),
        admin: Some(admin_cfg()),
        quality: None,
    };
    let state = AppState::new(pool, cfg);
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    Ok((addr, state))
}

async fn wait_for_kind(state: &AppState, kind: &str) -> bool {
    // The writer spawns a background task — poll briefly.
    for _ in 0..50 {
        let rows = state
            .event_log
            .list_recent(Some(kind), None, None, 10)
            .await
            .unwrap_or_default();
        if !rows.is_empty() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn writer_persists_session_ended() -> anyhow::Result<()> {
    let (_addr, state) = boot().await?;
    // The team_event_log table has a FK on user_id -> users(id); the writer
    // passes through the user_id from the event verbatim, so we seed a real
    // user first.
    let uid = seed_user(&state, "alice@acme.dev").await;
    state.event_log_writer.log(TeamEvent::SessionEnded {
        session_id: Uuid::new_v4(),
        user_id: uid,
        cwd: "/proj".into(),
        ts: time::OffsetDateTime::now_utc(),
    });
    assert!(
        wait_for_kind(&state, "session_ended").await,
        "team_event_log must contain a session_ended row"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn writer_persists_wiki_page_ready() -> anyhow::Result<()> {
    let (_addr, state) = boot().await?;
    let uid = seed_user(&state, "alice@acme.dev").await;
    state.event_log_writer.log(TeamEvent::WikiPageReady {
        page_id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        user_id: uid,
        cwd: "/proj".into(),
        title: "Setup".into(),
        ts: time::OffsetDateTime::now_utc(),
    });
    assert!(
        wait_for_kind(&state, "wiki_page_ready").await,
        "team_event_log must contain a wiki_page_ready row"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn writer_persists_skill_saved() -> anyhow::Result<()> {
    let (_addr, state) = boot().await?;
    let uid = seed_user(&state, "alice@acme.dev").await;
    state.event_log_writer.log(TeamEvent::SkillSaved {
        skill_id: Uuid::new_v4(),
        user_id: uid,
        name: "spec-to-tests".into(),
        ts: time::OffsetDateTime::now_utc(),
    });
    assert!(
        wait_for_kind(&state, "skill_saved").await,
        "team_event_log must contain a skill_saved row"
    );
    Ok(())
}
