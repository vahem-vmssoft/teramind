use teramind_core::ids::SessionId;
use teramind_db::repos::SkillObservationRepo;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upsert_merges_session_ids_and_bumps_frequency() -> anyhow::Result<()> {
    let pool = teramind_db::testing::fresh_pool().await?;
    let r = SkillObservationRepo::new(pool.clone());

    let sa = SessionId(Uuid::new_v4());
    let sb = SessionId(Uuid::new_v4());

    r.upsert("tool_chain", "sigA", &[sa], serde_json::json!({"k":"v"}))
        .await?;
    let obs1 = r.find_by_sig("tool_chain", "sigA").await?.unwrap();
    assert_eq!(obs1.frequency, 1);

    r.upsert("tool_chain", "sigA", &[sb], serde_json::json!({"k":"v"}))
        .await?;
    let obs2 = r.find_by_sig("tool_chain", "sigA").await?.unwrap();
    assert_eq!(obs2.frequency, 2);
    assert!(obs2.session_ids.contains(&sa.0) && obs2.session_ids.contains(&sb.0));

    // Same session twice does not double-count.
    r.upsert("tool_chain", "sigA", &[sa], serde_json::json!({"k":"v"}))
        .await?;
    let obs3 = r.find_by_sig("tool_chain", "sigA").await?.unwrap();
    assert_eq!(
        obs3.frequency, 2,
        "duplicate session must not increment frequency"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn list_open_filters_by_threshold() -> anyhow::Result<()> {
    let pool = teramind_db::testing::fresh_pool().await?;
    let r = SkillObservationRepo::new(pool.clone());

    for i in 0..3 {
        r.upsert(
            "tool_chain",
            &format!("sig{i}"),
            &[SessionId(Uuid::new_v4())],
            serde_json::json!({}),
        )
        .await?;
    }
    // One observation gets 3 sessions.
    let sigs_high = "sig0";
    r.upsert(
        "tool_chain",
        sigs_high,
        &[SessionId(Uuid::new_v4())],
        serde_json::json!({}),
    )
    .await?;
    r.upsert(
        "tool_chain",
        sigs_high,
        &[SessionId(Uuid::new_v4())],
        serde_json::json!({}),
    )
    .await?;

    let above = r.list_open(3, 10).await?;
    assert_eq!(above.len(), 1);
    assert_eq!(above[0].signature, "sig0");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mark_status_transitions() -> anyhow::Result<()> {
    let pool = teramind_db::testing::fresh_pool().await?;
    let r = SkillObservationRepo::new(pool.clone());

    r.upsert(
        "tool_chain",
        "sigX",
        &[SessionId(Uuid::new_v4())],
        serde_json::json!({}),
    )
    .await?;
    let obs = r.find_by_sig("tool_chain", "sigX").await?.unwrap();
    r.mark_synthesized(obs.id).await?;
    let after = r.find_by_sig("tool_chain", "sigX").await?.unwrap();
    assert_eq!(after.status, "synthesized");

    Ok(())
}
