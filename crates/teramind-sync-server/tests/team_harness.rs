//! End-to-end: spin up the server against an embedded PG, redeem an invite,
//! make several authenticated requests, verify rows landed correctly.

use ed25519_dalek::SigningKey;
use rand::{rngs::OsRng, RngCore};
use serde_json::json;
use std::net::SocketAddr;
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool, repos::InviteRepo};
use teramind_sync_server::config::*;
use teramind_sync_server::invite::InviteCode;
use teramind_sync_server::proof::{body_hash_hex, sign, token_hash_hex, ProofClaims};
use teramind_sync_server::server::build_router;
use teramind_sync_server::state::AppState;
use time::{Duration as TDur, OffsetDateTime};
use uuid::Uuid;

struct Client {
    addr: SocketAddr,
    token: String,
    sk: SigningKey,
}

impl Client {
    async fn redeem(addr: SocketAddr, pool: &DbPool, email: &str) -> Self {
        let invites = InviteRepo::new(pool.clone());
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        let sk = SigningKey::from_bytes(&seed);
        let pk = sk.verifying_key().to_bytes().to_vec();
        let code = InviteCode::generate(&mut OsRng);
        invites
            .create(
                &code.hash(),
                email,
                None,
                None,
                OffsetDateTime::now_utc() + TDur::days(7),
            )
            .await
            .unwrap();
        let r = reqwest::Client::new()
            .post(format!("http://{addr}/v1/auth/redeem"))
            .json(&json!({
                "invite_code": code.as_str(),
                "device_name": format!("{email}-dev"),
                "device_public_key_b64": base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD, &pk),
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let body: serde_json::Value = r.json().await.unwrap();
        Self {
            addr,
            token: body["device_token"].as_str().unwrap().into(),
            sk,
        }
    }

    fn signed_post(&self, path: &str, body: &[u8]) -> reqwest::RequestBuilder {
        let url = format!("http://{}{}", self.addr, path);
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let claims = ProofClaims {
            htm: "POST".into(),
            htu: url.clone(),
            iat: now,
            jti: format!("jti-{}", Uuid::new_v4()),
            ath: token_hash_hex(&self.token),
            bsh: body_hash_hex(body),
        };
        let proof = sign(&claims, &self.sk);
        reqwest::Client::new()
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("X-Teramind-Proof", proof)
            .header("Content-Type", "application/json")
            .body(body.to_vec())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_redeem_then_ingest_flow() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
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

    let alice = Client::redeem(addr, &pool, "alice@acme.dev").await;
    let bob = Client::redeem(addr, &pool, "bob@acme.dev").await;

    for client in [&alice, &bob] {
        let sid = Uuid::new_v4();
        let started = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let body = serde_json::to_vec(&json!({
            "events": [
                { "client_event_id": Uuid::new_v4().to_string(),
                  "ts": started.format(&time::format_description::well_known::Rfc3339).unwrap(),
                  "event": { "type": "session_start",
                             "session_id": sid.to_string(),
                             "agent_kind": "claude_code", "cwd": "/x",
                             "os": "linux", "hostname": "h", "user_login": "u",
                             "git_head": null, "git_branch": null, "agent_session_id": null } }
            ]
        }))?;
        let r = client.signed_post("/v1/ingest", &body).send().await?;
        assert_eq!(r.status(), 200);
    }

    let (total,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM sessions WHERE user_id IS NOT NULL")
            .fetch_one(pool.pg())
            .await?;
    assert_eq!(total, 2);

    let (alice_users,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM sessions s \
         JOIN users u ON u.id = s.user_id \
         WHERE u.email = 'alice@acme.dev'",
    )
    .fetch_one(pool.pg())
    .await?;
    assert_eq!(alice_users, 1);

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stolen_token_without_key_fails_403() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
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

    let alice = Client::redeem(addr, &pool, "alice@acme.dev").await;

    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let attacker_sk = SigningKey::from_bytes(&seed);
    let attacker = Client {
        addr,
        token: alice.token.clone(),
        sk: attacker_sk,
    };

    let body = serde_json::to_vec(&json!({ "events": [] }))?;
    let r = attacker.signed_post("/v1/ingest", &body).send().await?;
    assert_eq!(
        r.status(),
        403,
        "stolen token without matching private key must fail"
    );

    sup.shutdown().await?;
    Ok(())
}
