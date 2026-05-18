use teramind_db::repos::QualityRunRepo;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn insert_and_list_latest() -> anyhow::Result<()> {
    let pool = teramind_db::testing::fresh_pool().await?;
    let repo = QualityRunRepo::new(pool.clone());

    repo.insert(
        "lexical",
        None,
        0.142,
        0.301,
        0.230,
        0.180,
        0.420,
        42.0,
        380.0,
        100,
        500,
        serde_json::json!({}),
        serde_json::json!({"k":"v"}),
        "scheduled",
    )
    .await?;
    repo.insert(
        "semantic",
        Some("ollama:nomic-embed-text-v2-moe".into()),
        0.537,
        0.412,
        0.480,
        0.410,
        0.620,
        50.0,
        410.0,
        100,
        500,
        serde_json::json!({}),
        serde_json::json!({}),
        "scheduled",
    )
    .await?;

    let runs = repo.list_recent(None, 10).await?;
    assert_eq!(runs.len(), 2);
    let latest_semantic = repo.latest("semantic").await?;
    assert!(latest_semantic.is_some());
    assert_eq!(latest_semantic.unwrap().ndcg10, 0.537);

    Ok(())
}
