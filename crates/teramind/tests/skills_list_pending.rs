//! codifier §7.2 — `teramind skills list --filter pending` filters to pending
//! candidates (excludes approved/authored live skills).

#![cfg(unix)]
use teramind_core::ids::SessionId;
use teramind_db::repos::{SkillCandidateRepo, SkillObservationRepo, SkillRepo};

mod common;
use common::{boot_daemon, connect_daemon_db, stop_daemon};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn skills_list_pending_excludes_approved_live() {
    if std::env::var("TERAMIND_TEST_PG_URL").is_err() {
        eprintln!("skipping: TERAMIND_TEST_PG_URL unset");
        return;
    }
    let h = boot_daemon();
    let pool = connect_daemon_db(&h).await.expect("connect to daemon DB");

    // Approved (live) authored skill: should NOT appear under --filter pending.
    let approved_name = "approved-live-skill";
    SkillRepo::new(pool.clone())
        .upsert_authored(approved_name, "live one", "body-A")
        .await
        .unwrap();

    // Pending candidate: SHOULD appear.
    let obs_repo = SkillObservationRepo::new(pool.clone());
    let pending_name = "pending-cand-skill";
    obs_repo
        .upsert(
            "tool_chain",
            pending_name,
            &[SessionId::new()],
            serde_json::json!({"tool": "ls"}),
        )
        .await
        .unwrap();
    let obs = obs_repo
        .find_by_sig("tool_chain", pending_name)
        .await
        .unwrap()
        .unwrap();
    SkillCandidateRepo::new(pool.clone())
        .insert(
            obs.id,
            pending_name,
            "desc",
            "body-P",
            &["/workspace".into()],
            &[SessionId::new()],
            "test-model",
            5,
            10,
        )
        .await
        .unwrap();

    let out = h
        .cmd()
        .args(["skills", "list", "--filter", "pending"])
        .output()
        .expect("exec teramind skills list --filter pending");
    stop_daemon(&h);

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "exit non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains(pending_name),
        "stdout should contain pending candidate '{pending_name}':\n{stdout}"
    );
    assert!(
        !stdout.contains(approved_name),
        "stdout MUST NOT contain approved live skill '{approved_name}':\n{stdout}"
    );
}
