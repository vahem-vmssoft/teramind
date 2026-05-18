//! Verifies the skill-codifier migration applies cleanly.

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn migration_creates_observation_and_candidate_tables() -> anyhow::Result<()> {
    let pool = teramind_db::testing::fresh_pool().await?;

    for t in ["skill_observations", "skill_candidates"] {
        let (exists,): (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = $1)",
        )
        .bind(t)
        .fetch_one(pool.pg())
        .await?;
        assert!(exists, "table `{t}` must exist after migration");
    }

    let (exists,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
         WHERE table_name = 'skills' AND column_name = 'applies_to_cwds')",
    )
    .fetch_one(pool.pg())
    .await?;
    assert!(exists, "skills.applies_to_cwds must exist after migration");

    Ok(())
}
