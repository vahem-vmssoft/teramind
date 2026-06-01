//! `teramind team share-set [--enable|--disable]` — flip the per-project
//! team-share marker file directly, without round-tripping through the agent.

use anyhow::{bail, Result};
use teramind_core::team_share::{write_marker_at_cwd, ShareMarker};

pub async fn share_set(enable: bool, disable: bool) -> Result<()> {
    if enable == disable {
        bail!("exactly one of --enable or --disable must be passed");
    }
    let cwd = std::env::current_dir()?;
    let set_by = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".into());
    let marker = ShareMarker {
        share: enable,
        set_by,
        set_at: time::OffsetDateTime::now_utc(),
    };
    let path = write_marker_at_cwd(&cwd, &marker)?;
    println!(
        "team-share marker written: {} (share={})",
        path.display(),
        marker.share
    );
    Ok(())
}
