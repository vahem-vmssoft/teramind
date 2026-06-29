mod common;

use common::Harness;
use teramind_core::ids::{ClientEventId, SessionId};
use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
use time::OffsetDateTime;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn file_save_to_row_p99_under_one_second() -> anyhow::Result<()> {
    let h = Harness::start().await?;
    let proj = h._tmp.path().canonicalize()?.join("proj");
    std::fs::create_dir_all(&proj)?;
    std::fs::write(proj.join("a.rs"), "v0\n")?;

    let sid = SessionId::new();
    h.ingest
        .try_enqueue(EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::now_utc(),
            event: IngestEvent::SessionStart {
                session_id: sid,
                agent_session_id: None,
                agent_kind: "claude_code".into(),
                agent_version: None,
                cwd: proj.to_string_lossy().to_string(),
                os: "linux".into(),
                hostname: "h".into(),
                user_login: "u".into(),
                git_head: None,
                git_branch: None,
            },
        })
        .map_err(|_| anyhow::anyhow!("enqueue"))?;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let n = 20;
    let mut latencies = Vec::with_capacity(n);
    for i in 1..=n {
        let started = std::time::Instant::now();
        std::fs::write(proj.join("a.rs"), format!("v{i}\n"))?;
        // Poll until count_diffs == i.
        loop {
            let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM file_diffs")
                .fetch_one(h.pool.pg())
                .await?;
            if count as usize >= i {
                latencies.push(started.elapsed());
                break;
            }
            if started.elapsed() > std::time::Duration::from_secs(3) {
                anyhow::bail!("timeout waiting for diff #{i}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
    latencies.sort();
    let p99_idx = ((latencies.len() as f64 * 0.99) as usize).min(latencies.len() - 1);
    let p99 = latencies[p99_idx];
    assert!(
        p99 < std::time::Duration::from_secs(1),
        "p99 = {p99:?}, budget 1 s (spec §9.7)"
    );
    Ok(())
}
