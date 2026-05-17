use teramind_db::repos::{SkillCandidateRepo, SkillObservationRepo};
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};
use teramind_core::ids::SessionId;
use uuid::Uuid;

async fn pool_with_migrations() -> anyhow::Result<(tempfile::TempDir, teramind_db::pg_supervisor::PgSupervisor, DbPool)> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;
    Ok((dir, sup, pool))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn insert_then_list_pending() -> anyhow::Result<()> {
    let (_dir, sup, pool) = pool_with_migrations().await?;
    let obs = SkillObservationRepo::new(pool.clone());
    let cand = SkillCandidateRepo::new(pool.clone());

    obs.upsert("tool_chain", "sig1", &[SessionId(Uuid::new_v4())], serde_json::json!({})).await?;
    let o = obs.find_by_sig("tool_chain", "sig1").await?.unwrap();

    cand.insert(
        o.id, "rust-pr-prep", "Build + test + commit", "# rust-pr-prep\n…",
        &["/openvms-*".into()],
        &[SessionId(Uuid::new_v4())],
        "ollama:qwen3.6:latest", 1200, 350,
    ).await?;

    let pending = cand.list_pending(10).await?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].name, "rust-pr-prep");

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn approve_then_list_approved() -> anyhow::Result<()> {
    let (_dir, sup, pool) = pool_with_migrations().await?;
    let obs = SkillObservationRepo::new(pool.clone());
    let cand = SkillCandidateRepo::new(pool.clone());

    obs.upsert("tool_chain", "sig1", &[SessionId(Uuid::new_v4())], serde_json::json!({})).await?;
    let o = obs.find_by_sig("tool_chain", "sig1").await?.unwrap();
    cand.insert(o.id, "n", "d", "b", &[], &[], "m", 0, 0).await?;
    let pending = cand.list_pending(10).await?;
    let id = pending[0].id;

    // Approval is just SQL UPDATE per spec §3.
    sqlx::query("UPDATE skill_candidates SET status='approved', reviewer='admin', reviewed_at=now() WHERE id=$1")
        .bind(id.0).execute(pool.pg()).await?;

    let approved = cand.list_approved(10).await?;
    assert_eq!(approved.len(), 1);
    assert_eq!(approved[0].id, id);

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unique_pending_name_constraint() -> anyhow::Result<()> {
    let (_dir, sup, pool) = pool_with_migrations().await?;
    let obs = SkillObservationRepo::new(pool.clone());
    let cand = SkillCandidateRepo::new(pool.clone());

    obs.upsert("tool_chain", "sigA", &[SessionId(Uuid::new_v4())], serde_json::json!({})).await?;
    let oa = obs.find_by_sig("tool_chain", "sigA").await?.unwrap();
    cand.insert(oa.id, "dup-name", "d", "b", &[], &[], "m", 0, 0).await?;

    obs.upsert("tool_chain", "sigB", &[SessionId(Uuid::new_v4())], serde_json::json!({})).await?;
    let ob = obs.find_by_sig("tool_chain", "sigB").await?.unwrap();
    let res = cand.insert(ob.id, "dup-name", "d", "b", &[], &[], "m", 0, 0).await;
    assert!(res.is_err(), "second pending with same name must fail unique constraint");

    sup.shutdown().await?;
    Ok(())
}
