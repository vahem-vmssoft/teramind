//! Dashboard §6 — eval failures (binary missing / non-zero exit / JSON parse
//! error) must record a quality_runs row with an `error` indicator in
//! raw_json so the dashboard can surface scheduler health.

use teramind_db::repos::QualityRunRepo;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn missing_binary_persists_failure_row() -> anyhow::Result<()> {
    let pool = teramind_db::testing::fresh_pool().await?;
    let repo = QualityRunRepo::new(pool.clone());

    // Point at a path that cannot exist on disk — Command::output should
    // surface a spawn error (ENOENT), and run_one must persist a failure row.
    let bogus_binary = "/tmp/teramind-does-not-exist-eval-XYZ-aaaaaaa";
    teramind_sync_server::quality_scheduler::run_one(&repo, bogus_binary, "lexical").await;

    let rows = repo.list_recent(Some("lexical"), 10).await?;
    assert_eq!(
        rows.len(),
        1,
        "exactly one row must be persisted for the failed run"
    );
    let row = &rows[0];
    assert_eq!(row.source, "scheduled");
    // raw_json must carry an `error` indicator.
    let raw = &row.raw_json;
    assert!(
        raw.get("error").is_some(),
        "raw_json must include an `error` field describing the failure (got: {raw})"
    );
    // Sentinel metrics: 0.0 (finite, so downstream validators don't trip on NaN).
    assert!(row.ndcg10.is_finite(), "metrics must remain finite");
    Ok(())
}
