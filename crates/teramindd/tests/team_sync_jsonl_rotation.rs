//! team-sync §2.1: forwarder resumes from persisted offset across daily JSONL
//! rotation with zero event loss, and re-spawn after crash does not duplicate.

use axum::{routing::post, Json, Router};
use ed25519_dalek::SigningKey;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use teramind_core::ids::SessionId;
use teramind_core::team::TeamConfig;
use teramindd::services::decision_cache::{DecisionCache, ShareDecision};
use teramindd::services::sync_offset::SyncOffset;
use teramindd::services::team_sync::{TeamSync, TeamSyncDeps};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Default, Clone)]
struct Received {
    client_event_ids: Arc<Mutex<Vec<String>>>,
}

async fn ingest_handler(
    axum::extract::State(state): axum::extract::State<Received>,
    Json(body): Json<Value>,
) -> axum::http::StatusCode {
    if let Some(events) = body.get("events").and_then(|v| v.as_array()) {
        let mut buf = state.client_event_ids.lock().unwrap();
        for ev in events {
            if let Some(id) = ev.get("client_event_id").and_then(|v| v.as_str()) {
                buf.push(id.to_string());
            }
        }
    }
    axum::http::StatusCode::OK
}

async fn boot_mock_server() -> anyhow::Result<(SocketAddr, Received)> {
    let received = Received::default();
    let app = Router::new()
        .route("/v1/ingest", post(ingest_handler))
        .with_state(received.clone());
    let listener = tokio::net::TcpListener::bind::<SocketAddr>("127.0.0.1:0".parse()?).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Ok((addr, received))
}

fn make_envelope(sid: Uuid, kind: &str) -> (String, String) {
    let client_event_id = Uuid::new_v4().to_string();
    let ts = OffsetDateTime::from_unix_timestamp(1_700_000_000)
        .unwrap()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    let env = if kind == "session_start" {
        json!({
            "client_event_id": client_event_id, "ts": ts,
            "event": { "type": "session_start", "session_id": sid.to_string(),
                       "agent_kind": "claude_code", "cwd": "/p",
                       "os": "linux", "hostname": "h", "user_login": "u",
                       "git_head": null, "git_branch": null, "agent_session_id": null }
        })
    } else {
        json!({
            "client_event_id": client_event_id, "ts": ts,
            "event": { "type": "user_prompt", "session_id": sid.to_string(),
                       "turn_ordinal": 0, "prompt": format!("p-{kind}"), "turn_id": null }
        })
    };
    (client_event_id, serde_json::to_string(&env).unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forwarder_resumes_across_rotation_without_loss() -> anyhow::Result<()> {
    let (addr, received) = boot_mock_server().await?;
    let raw_dir = tempfile::tempdir()?;

    let sid = Uuid::new_v4();
    // Two daily files.
    let day1_path = raw_dir.path().join("2026-05-31.jsonl");
    let day2_path = raw_dir.path().join("2026-06-01.jsonl");

    let (id_a, line_a) = make_envelope(sid, "session_start");
    let (id_b, line_b) = make_envelope(sid, "p1");
    let (id_c, line_c) = make_envelope(sid, "p2");
    let (id_d, line_d) = make_envelope(sid, "p3");

    std::fs::write(&day1_path, format!("{line_a}\n{line_b}\n"))?;
    // Ensure day2 has a strictly-greater mtime so select_jsonl_file picks it
    // after day1 is fully consumed.
    tokio::time::sleep(Duration::from_millis(50)).await;
    std::fs::write(&day2_path, format!("{line_c}\n{line_d}\n"))?;

    // Start the offset pointing at the FIRST file with byte_offset=0.
    let initial_off = SyncOffset {
        file: Some("2026-05-31.jsonl".into()),
        byte_offset: 0,
    };
    initial_off.save(raw_dir.path())?;

    let cache = DecisionCache::new();
    cache.set_initial(SessionId(sid), ShareDecision::Allowed);

    let team_cfg = TeamConfig {
        server_url: format!("http://{addr}"),
        user_email: "u@e".into(),
        user_id: Uuid::new_v4().to_string(),
        device_id: Uuid::new_v4().to_string(),
        device_token: "tok".into(),
        device_name: "d".into(),
        redeemed_at: OffsetDateTime::now_utc(),
    };
    let sk = SigningKey::from_bytes(&[7u8; 32]);

    let forwarder = TeamSync::spawn(TeamSyncDeps {
        team_cfg: Arc::new(team_cfg.clone()),
        signing_key: Arc::new(sk.clone()),
        raw_dir: raw_dir.path().to_path_buf(),
        cache: cache.clone(),
        poll_interval: Duration::from_millis(100),
        batch_size: 8,
        max_attempts: 3,
    });

    // Wait for all 4 to ship.
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let n = received.client_event_ids.lock().unwrap().len();
        if n >= 4 {
            break;
        }
    }

    let shipped_after_first = received.client_event_ids.lock().unwrap().clone();
    assert!(
        shipped_after_first.contains(&id_a)
            && shipped_after_first.contains(&id_b)
            && shipped_after_first.contains(&id_c)
            && shipped_after_first.contains(&id_d),
        "all 4 events from both files must be shipped, got: {:?}",
        shipped_after_first
    );

    // Now simulate a crash: drop the forwarder.
    drop(forwarder);

    let count_at_crash = received.client_event_ids.lock().unwrap().len();

    // Re-spawn against the SAME raw_dir/offset and assert no duplicates ship.
    let _forwarder2 = TeamSync::spawn(TeamSyncDeps {
        team_cfg: Arc::new(team_cfg),
        signing_key: Arc::new(sk),
        raw_dir: raw_dir.path().to_path_buf(),
        cache: cache.clone(),
        poll_interval: Duration::from_millis(100),
        batch_size: 8,
        max_attempts: 3,
    });

    tokio::time::sleep(Duration::from_millis(1000)).await;
    let final_ids = received.client_event_ids.lock().unwrap().clone();
    assert_eq!(
        final_ids.len(),
        count_at_crash,
        "re-spawn must not re-ship already-acked events; got delta: {:?}",
        &final_ids[count_at_crash.min(final_ids.len())..]
    );

    Ok(())
}
