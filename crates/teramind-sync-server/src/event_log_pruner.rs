//! Periodic delete of old team_event_log rows.

use std::time::Duration;
use teramind_db::repos::TeamEventLogRepo;
use tracing::{info, warn};

pub fn spawn(repo: TeamEventLogRepo, retention_days: i64, interval: Duration) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        tick.tick().await; // burn the immediate-fire tick
        loop {
            tick.tick().await;
            match repo.prune_older_than(retention_days).await {
                Ok(n) if n > 0 => info!(rows = n, "event_log pruned"),
                Ok(_) => {}
                Err(e) => warn!(error = %e, "event_log prune failed"),
            }
        }
    });
}
