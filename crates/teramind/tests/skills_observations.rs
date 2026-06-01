//! codifier §7.2 — `teramind skills observations --kind <K> --min-freq <N>`
//! filters observations to the matching kind/freq.

#![cfg(unix)]
use teramind_core::ids::SessionId;
use teramind_db::repos::SkillObservationRepo;

mod common;
use common::{boot_daemon, connect_daemon_db, stop_daemon};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn skills_observations_filters_by_kind_and_min_freq() {
    if std::env::var("TERAMIND_TEST_PG_URL").is_err() {
        eprintln!("skipping: TERAMIND_TEST_PG_URL unset");
        return;
    }
    let h = boot_daemon();
    let pool = connect_daemon_db(&h).await.expect("connect to daemon DB");

    let obs = SkillObservationRepo::new(pool.clone());

    // tool_chain @ frequency 5 — should appear.
    let tc_sids: Vec<SessionId> = (0..5).map(|_| SessionId::new()).collect();
    obs.upsert(
        "tool_chain",
        "tc-signature-A",
        &tc_sids,
        serde_json::json!({"tool": "rg"}),
    )
    .await
    .unwrap();

    // problem_fix @ frequency 2 — should NOT appear (wrong kind, also below min-freq).
    let pf_sids: Vec<SessionId> = (0..2).map(|_| SessionId::new()).collect();
    obs.upsert(
        "problem_fix",
        "pf-signature-B",
        &pf_sids,
        serde_json::json!({"err": "EACCES"}),
    )
    .await
    .unwrap();

    let out = h
        .cmd()
        .args([
            "skills",
            "observations",
            "--kind",
            "tool_chain",
            "--min-freq",
            "3",
        ])
        .output()
        .expect("exec teramind skills observations");
    stop_daemon(&h);

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "exit non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("tc-signature-A"),
        "stdout should contain tool_chain signature:\n{stdout}"
    );
    assert!(
        !stdout.contains("pf-signature-B"),
        "stdout MUST NOT contain problem_fix signature:\n{stdout}"
    );
}
