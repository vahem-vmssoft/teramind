//! Two developers, one server. Alice's forwarder ships a session; Bob's
//! HttpsTransport finds it via /v1/rpc.

use ed25519_dalek::SigningKey;
use rand::RngExt;
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use teramind_core::ids::SessionId;
use teramind_core::team::TeamConfig;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool, repos::InviteRepo};
use teramind_ipc::proto::{Request, Response};
use teramind_ipc::rpc_transport::RpcTransport;
use teramind_mcp::transport_https::HttpsTransport;
use teramind_sync_server::{config::*, invite::InviteCode, server::build_router, state::AppState};
use teramindd::services::decision_cache::{DecisionCache, ShareDecision};
use teramindd::services::team_sync::{TeamSync, TeamSyncDeps};
use time::{Duration as TDur, OffsetDateTime};
use uuid::Uuid;

async fn redeem(
    addr: SocketAddr,
    pool: &DbPool,
    email: &str,
) -> anyhow::Result<(Arc<TeamConfig>, Arc<SigningKey>)> {
    let invites = InviteRepo::new(pool.clone());
    let mut seed = [0u8; 32];
    rand::rng().fill(&mut seed[..]);
    let sk = SigningKey::from_bytes(&seed);
    let pk = sk.verifying_key().to_bytes().to_vec();
    let code = InviteCode::generate(&mut rand::rng());
    invites
        .create(
            &code.hash(),
            email,
            None,
            None,
            OffsetDateTime::now_utc() + TDur::days(7),
        )
        .await?;
    let r = reqwest::Client::new()
        .post(format!("http://{addr}/v1/auth/redeem"))
        .json(&json!({
            "invite_code": code.as_str(),
            "device_name": email,
            "device_public_key_b64": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD, &pk),
        }))
        .send()
        .await?;
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await?;
    let cfg = TeamConfig {
        server_url: format!("http://{addr}"),
        user_email: email.into(),
        user_id: body["user_id"].as_str().unwrap().into(),
        device_id: body["device_id"].as_str().unwrap().into(),
        device_token: body["device_token"].as_str().unwrap().into(),
        device_name: email.into(),
        redeemed_at: OffsetDateTime::now_utc(),
    };
    Ok((Arc::new(cfg), Arc::new(sk)))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn alice_captures_bob_searches() -> anyhow::Result<()> {
    let pg_dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(pg_dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;
    let cfg = ServerConfig {
        listen_addr: "127.0.0.1:0".into(),
        database_url: "ignored".into(),
        tls: None,
        auth: AuthConfig::default(),
        ingest: IngestConfig::default(),
        admin: None,
        quality: None,
    };
    let state = AppState::new(pool.clone(), cfg);
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let (alice_cfg, alice_sk) = redeem(addr, &pool, "alice@acme.dev").await?;
    let (bob_cfg, bob_sk) = redeem(addr, &pool, "bob@acme.dev").await?;

    // Alice writes a session_start + user_prompt to her JSONL; forwarder ships.
    let raw_dir = tempfile::tempdir()?;
    let jsonl = raw_dir.path().join("2026-05-17.jsonl");
    let sid = Uuid::new_v4();
    let started = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let session_envelope = json!({
        "client_event_id": Uuid::new_v4().to_string(),
        "ts": started.format(&time::format_description::well_known::Rfc3339)?,
        "event": { "type": "session_start", "session_id": sid.to_string(),
                   "agent_kind": "claude_code", "cwd": "/x",
                   "os": "linux", "hostname": "h", "user_login": "u",
                   "git_head": null, "git_branch": null, "agent_session_id": null }
    });
    let prompt_envelope = json!({
        "client_event_id": Uuid::new_v4().to_string(),
        "ts": started.format(&time::format_description::well_known::Rfc3339)?,
        "event": { "type": "user_prompt", "session_id": sid.to_string(),
                   "turn_ordinal": 0, "prompt": "openvms fork autoconf probe" }
    });
    std::fs::write(
        &jsonl,
        format!(
            "{}\n{}\n",
            serde_json::to_string(&session_envelope)?,
            serde_json::to_string(&prompt_envelope)?,
        ),
    )?;
    let cache = DecisionCache::new();
    cache.set_initial(SessionId(sid), ShareDecision::Allowed);
    let _forwarder = TeamSync::spawn(TeamSyncDeps {
        team_cfg: alice_cfg.clone(),
        signing_key: alice_sk.clone(),
        raw_dir: raw_dir.path().to_path_buf(),
        cache: cache.clone(),
        poll_interval: Duration::from_millis(100),
        batch_size: 8,
        max_attempts: 3,
    });

    // Wait for the turn to land server-side.
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM turns WHERE session_id = $1")
            .bind(sid)
            .fetch_one(pool.pg())
            .await?;
        if n >= 1 {
            break;
        }
    }

    // The traces_fts materialized view must be refreshed manually before FTS search.
    // In production the daemon runs a periodic REFRESH; in tests we trigger it directly.
    sqlx::query("REFRESH MATERIALIZED VIEW traces_fts")
        .execute(pool.pg())
        .await?;

    // Bob queries via HttpsTransport.
    let bob_transport = HttpsTransport::new(bob_cfg, bob_sk);
    let r = bob_transport
        .request(Request::Search(teramind_core::types::SearchRequest {
            query: "fork autoconf".into(),
            limit: 10,
        }))
        .await?;
    match r {
        Response::SearchResults(s) => {
            assert!(
                !s.hits.is_empty(),
                "Bob must find Alice's content via team-wide search; got 0 hits"
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }

    sup.shutdown().await?;
    Ok(())
}
