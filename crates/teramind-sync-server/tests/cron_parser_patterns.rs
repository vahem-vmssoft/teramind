//! Dashboard §6 — the cron wrapper used by quality_scheduler must compute
//! correct next-fire times for the canonical patterns we document.
//!
//! quality_scheduler prepends a `"0 "` to the user-supplied 5-field crontab
//! so the underlying `cron` crate (which expects 6 fields: sec min hr dom mon
//! dow) interprets `0 2 * * *` as "every day at 02:00 UTC". This test mirrors
//! that wrapping and asserts the upcoming() iterator returns the expected
//! window.

use chrono::{TimeZone, Utc};
use cron::Schedule;
use std::str::FromStr;

fn schedule_from_5field(s: &str) -> Schedule {
    Schedule::from_str(&format!("0 {s}")).expect("valid cron")
}

#[test]
fn daily_2am_fires_within_24h() {
    let schedule = schedule_from_5field("0 2 * * *");
    let base = Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap();
    let next = schedule
        .after(&base)
        .next()
        .expect("daily schedule must yield a next fire time");
    assert!(next > base, "next fire must be in the future");
    let delta = next - base;
    assert!(
        delta < chrono::Duration::hours(24),
        "next daily 02:00 must be within 24h of 12:00 base, got {delta:?}"
    );
    // Specifically: from 12:00, the next 02:00 is 14h away.
    assert_eq!(
        next,
        Utc.with_ymd_and_hms(2026, 6, 2, 2, 0, 0).unwrap(),
        "expected next fire at 2026-06-02 02:00 UTC"
    );
}

#[test]
fn every_15_minutes_fires_within_15min() {
    let schedule = schedule_from_5field("*/15 * * * *");
    // Use a base time NOT on a 15-minute boundary so we exercise the rounding.
    let base = Utc.with_ymd_and_hms(2026, 6, 1, 12, 7, 30).unwrap();
    let next = schedule
        .after(&base)
        .next()
        .expect("*/15 schedule must yield a next fire time");
    assert!(next > base);
    let delta = next - base;
    assert!(
        delta <= chrono::Duration::minutes(15),
        "next */15 fire must be within 15 minutes of 12:07:30 base, got {delta:?}"
    );
    // From 12:07:30, the next */15 boundary is 12:15:00.
    assert_eq!(next, Utc.with_ymd_and_hms(2026, 6, 1, 12, 15, 0).unwrap(),);
}

#[test]
fn hourly_top_of_hour_fires_within_60min() {
    let schedule = schedule_from_5field("0 * * * *");
    let base = Utc.with_ymd_and_hms(2026, 6, 1, 9, 17, 0).unwrap();
    let next = schedule.after(&base).next().unwrap();
    let delta = next - base;
    assert!(delta <= chrono::Duration::minutes(60));
    assert_eq!(next, Utc.with_ymd_and_hms(2026, 6, 1, 10, 0, 0).unwrap());
}

#[test]
fn invalid_cron_pattern_fails_to_parse() {
    // The wrapper would warn and disable the scheduler — surface that here.
    let result = Schedule::from_str("0 not-a-cron-expression");
    assert!(result.is_err(), "invalid cron must fail to parse");
}
