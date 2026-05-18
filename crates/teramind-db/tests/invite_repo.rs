use teramind_db::repos::{DeviceRepo, InviteRepo, UserRepo};
use time::{Duration, OffsetDateTime};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn create_and_find_redeemable() -> anyhow::Result<()> {
    let pool = teramind_db::testing::fresh_pool().await?;
    let invites = InviteRepo::new(pool.clone());

    let code_hash = vec![0x10u8; 32];
    let exp = OffsetDateTime::now_utc() + Duration::days(7);
    invites
        .create(
            &code_hash,
            "alice@acme.dev",
            Some("Alice K."),
            Some("admin@acme.dev"),
            exp,
        )
        .await?;
    let found = invites
        .find_redeemable(&code_hash)
        .await?
        .expect("redeemable invite must be findable");
    assert_eq!(found.invited_email, "alice@acme.dev");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn redeemed_invite_is_no_longer_redeemable() -> anyhow::Result<()> {
    let pool = teramind_db::testing::fresh_pool().await?;
    let users = UserRepo::new(pool.clone());
    let devices = DeviceRepo::new(pool.clone());
    let invites = InviteRepo::new(pool.clone());

    let code_hash = vec![0x20u8; 32];
    let exp = OffsetDateTime::now_utc() + Duration::days(7);
    invites
        .create(&code_hash, "bob@acme.dev", None, None, exp)
        .await?;
    let u = users.upsert_by_email("bob@acme.dev", None).await?;
    let d = devices
        .insert(u.id, "bob-laptop", &[0x21u8; 32], &[0x22u8; 32])
        .await?;
    invites.mark_redeemed(&code_hash, d.id).await?;
    assert!(invites.find_redeemable(&code_hash).await?.is_none());

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_invite_is_not_redeemable() -> anyhow::Result<()> {
    let pool = teramind_db::testing::fresh_pool().await?;
    let invites = InviteRepo::new(pool.clone());
    let code_hash = vec![0x30u8; 32];
    let exp_past = OffsetDateTime::now_utc() - Duration::seconds(1);
    invites
        .create(&code_hash, "carol@acme.dev", None, None, exp_past)
        .await?;
    assert!(invites.find_redeemable(&code_hash).await?.is_none());
    Ok(())
}
