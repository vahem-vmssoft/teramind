//! codifier §7.2 — `teramind skills show <name|id>` prints skill name + body.

#![cfg(unix)]
use teramind_db::repos::SkillRepo;

mod common;
use common::{boot_daemon, connect_daemon_db, stop_daemon};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn skills_show_prints_name_and_body() {
    if std::env::var("TERAMIND_TEST_PG_URL").is_err() {
        eprintln!("skipping: TERAMIND_TEST_PG_URL unset");
        return;
    }
    let h = boot_daemon();
    let pool = connect_daemon_db(&h).await.expect("connect to daemon DB");

    let name = "my-skill";
    let body = format!("UNIQUE-BODY-MARKER-{}", uuid::Uuid::new_v4());
    SkillRepo::new(pool.clone())
        .upsert_authored(name, "desc", &body)
        .await
        .unwrap();

    let out = h
        .cmd()
        .args(["skills", "show", name])
        .output()
        .expect("exec teramind skills show my-skill");
    stop_daemon(&h);

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "exit non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains(name),
        "stdout should contain skill name '{name}':\n{stdout}"
    );
    assert!(
        stdout.contains(&body),
        "stdout should contain skill body marker:\n{stdout}"
    );
}
