use std::path::PathBuf;
use std::time::Duration;
use teramind_db::repos::storage_stats::{Sample, StorageStatsRepo};
use tracing::warn;

pub fn spawn(repo: StorageStatsRepo, raw_dir: PathBuf, database: String, interval: Duration) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if let Err(e) = tick(&repo, &raw_dir, &database).await {
                warn!(error = %e, "storage_stats sampler tick failed");
            }
        }
    });
}

async fn tick(repo: &StorageStatsRepo, raw_dir: &PathBuf, database: &str) -> anyhow::Result<()> {
    let jsonl_bytes = walk_dir_bytes(raw_dir).unwrap_or(0);
    let pg_bytes = repo.pg_database_size(database).await?;
    let s = Sample {
        pg_bytes,
        jsonl_bytes,
        session_count: repo.count_sessions().await?,
        turn_count:    repo.count_turns().await?,
        diff_count:    repo.count_diffs().await?,
    };
    repo.insert(s).await?;
    Ok(())
}

fn walk_dir_bytes(p: &PathBuf) -> std::io::Result<i64> {
    let mut total: i64 = 0;
    for entry in std::fs::read_dir(p)? {
        let entry = entry?;
        let md = entry.metadata()?;
        if md.is_file() { total += md.len() as i64; }
    }
    Ok(total)
}
