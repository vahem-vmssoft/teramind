//! Periodic refresh of the traces_fts materialized view.

use std::time::Duration;
use teramind_db::pool::DbPool;
use tracing::{info, warn};

pub fn spawn(pool: DbPool, interval: Duration) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        tick.tick().await; // first tick fires immediately
        loop {
            tick.tick().await;
            match sqlx::query("REFRESH MATERIALIZED VIEW CONCURRENTLY traces_fts")
                .execute(pool.pg())
                .await
            {
                Ok(_) => info!("traces_fts refreshed"),
                Err(e) => warn!(error = %e, "traces_fts refresh failed"),
            }
        }
    });
}
