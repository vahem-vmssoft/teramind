//! Admin subcommand bodies (invite, member).

use crate::config::ServerConfig;
use crate::invite::InviteCode;
use anyhow::Context;
use rand::rngs::OsRng;
use teramind_core::ids::{DeviceId, InviteId, UserId};
use teramind_db::pool::DbPool;
use teramind_db::repos::{DeviceRepo, InviteRepo, UserRepo};
use time::{Duration, OffsetDateTime};

pub struct AdminCtx {
    pub pool: DbPool,
    pub cfg: ServerConfig,
}

impl AdminCtx {
    pub async fn open(cfg: ServerConfig) -> anyhow::Result<Self> {
        let pool = DbPool::connect_url(&cfg.database_url).await?;
        Ok(Self { pool, cfg })
    }
}

pub async fn invite_create(
    ctx: &AdminCtx,
    email: &str,
    display_name: Option<&str>,
    created_by: Option<&str>,
    expires_in_days: Option<i64>,
) -> anyhow::Result<()> {
    let invites = InviteRepo::new(ctx.pool.clone());
    let days = expires_in_days.unwrap_or(ctx.cfg.auth.invite_default_expires_days);
    let expires_at = OffsetDateTime::now_utc() + Duration::days(days);
    let code = InviteCode::generate(&mut OsRng);
    invites
        .create(&code.hash(), email, display_name, created_by, expires_at)
        .await
        .context("create invite")?;
    println!("invite created:");
    println!("  code:    {}", code.as_str());
    println!("  email:   {email}");
    println!("  expires: {expires_at}");
    if let Some(by) = created_by {
        println!("  by:      {by}");
    }
    Ok(())
}

pub async fn invite_list(ctx: &AdminCtx) -> anyhow::Result<()> {
    let invites = InviteRepo::new(ctx.pool.clone()).list_outstanding().await?;
    if invites.is_empty() {
        println!("no outstanding invites");
        return Ok(());
    }
    println!("{:<36}  {:<30}  {:<25}", "id", "email", "expires_at");
    for i in invites {
        println!("{:<36}  {:<30}  {}", i.id.0, i.invited_email, i.expires_at);
    }
    Ok(())
}

pub async fn invite_revoke(ctx: &AdminCtx, id_str: &str) -> anyhow::Result<()> {
    let id = InviteId(uuid::Uuid::parse_str(id_str).context("bad uuid")?);
    InviteRepo::new(ctx.pool.clone()).revoke(id).await?;
    println!("invite {id_str} revoked");
    Ok(())
}

pub async fn member_list(ctx: &AdminCtx) -> anyhow::Result<()> {
    let users = UserRepo::new(ctx.pool.clone()).list_all().await?;
    let devices = DeviceRepo::new(ctx.pool.clone());
    println!(
        "{:<36}  {:<30}  {:>7}  {:<25}",
        "user_id", "email", "devices", "last_seen"
    );
    for u in users {
        let ds = devices.list_for_user(u.id).await?;
        let last = ds.iter().filter_map(|d| d.last_seen_at).max();
        println!(
            "{:<36}  {:<30}  {:>7}  {}",
            u.id.0,
            u.email,
            ds.len(),
            last.map(|t| t.to_string()).unwrap_or_else(|| "—".into())
        );
    }
    Ok(())
}

pub async fn member_revoke_device(ctx: &AdminCtx, id_str: &str) -> anyhow::Result<()> {
    let id = DeviceId(uuid::Uuid::parse_str(id_str).context("bad uuid")?);
    DeviceRepo::new(ctx.pool.clone()).revoke(id).await?;
    println!("device {id_str} revoked");
    Ok(())
}

pub async fn member_revoke_user(ctx: &AdminCtx, id_str: &str) -> anyhow::Result<()> {
    let id = UserId(uuid::Uuid::parse_str(id_str).context("bad uuid")?);
    UserRepo::new(ctx.pool.clone()).revoke(id).await?;
    println!(
        "user {id_str} revoked (cascade: associated devices remain rows but auth lookups now fail)"
    );
    Ok(())
}
