use teramind_db::repos::{TeamEventLogRepo, UserRepo};
use teramind_db::{migrate, pg_supervisor::PgSupervisor, pool::DbPool};

async fn fresh_pool() -> anyhow::Result<(tempfile::TempDir, teramind_db::pg_supervisor::PgSupervisor, DbPool)> {
    let dir = tempfile::tempdir()?;
    let sup = PgSupervisor::start(dir.path().to_path_buf(), "teramind").await?;
    let pool = DbPool::connect(sup.connect_options()).await?;
    migrate::run(&pool).await?;
    Ok((dir, sup, pool))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn insert_and_list_recent_roundtrips() -> anyhow::Result<()> {
    let (_d, sup, pool) = fresh_pool().await?;
    let users = UserRepo::new(pool.clone());
    let log = TeamEventLogRepo::new(pool.clone());
    let u = users.upsert_by_email("alice@acme.dev", None).await?;

    log.insert("session_ended", Some(u.id), Some("/proj".into()),
               serde_json::json!({"session_id":"abc"})).await?;
    log.insert("skill_saved", Some(u.id), None,
               serde_json::json!({"skill_id":"x"})).await?;

    let rows = log.list_recent(None, None, None, 100).await?;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].kind, "skill_saved");      // newest first
    assert_eq!(rows[1].kind, "session_ended");

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn list_recent_filters_by_kind_and_user() -> anyhow::Result<()> {
    let (_d, sup, pool) = fresh_pool().await?;
    let users = UserRepo::new(pool.clone());
    let log = TeamEventLogRepo::new(pool.clone());
    let alice = users.upsert_by_email("a@x.dev", None).await?;
    let bob   = users.upsert_by_email("b@x.dev", None).await?;

    log.insert("session_ended", Some(alice.id), None, serde_json::json!({})).await?;
    log.insert("skill_saved",   Some(alice.id), None, serde_json::json!({})).await?;
    log.insert("session_ended", Some(bob.id),   None, serde_json::json!({})).await?;

    let alice_rows = log.list_recent(None, None, Some(alice.id), 10).await?;
    assert_eq!(alice_rows.len(), 2);
    let alice_ended = log.list_recent(Some("session_ended"), None, Some(alice.id), 10).await?;
    assert_eq!(alice_ended.len(), 1);

    sup.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn prune_deletes_old_rows() -> anyhow::Result<()> {
    let (_d, sup, pool) = fresh_pool().await?;
    let log = TeamEventLogRepo::new(pool.clone());

    log.insert("session_ended", None, None, serde_json::json!({})).await?;
    // Backdate it 100 days.
    sqlx::query("UPDATE team_event_log SET ts = now() - interval '100 days'")
        .execute(pool.pg()).await?;

    let deleted = log.prune_older_than(90).await?;
    assert_eq!(deleted, 1);
    assert!(log.list_recent(None, None, None, 10).await?.is_empty());

    sup.shutdown().await?;
    Ok(())
}
