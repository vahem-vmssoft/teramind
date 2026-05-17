//! Per-IP login attempt throttle. 5 failures in 60s → 5-min lockout.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_FAILURES: u8 = 5;
const WINDOW: Duration = Duration::from_secs(60);
const LOCKOUT: Duration = Duration::from_secs(300);

#[derive(Default)]
struct Entry {
    failures: Vec<Instant>,
    locked_until: Option<Instant>,
}

pub struct LoginThrottle {
    inner: Mutex<HashMap<IpAddr, Entry>>,
}

impl LoginThrottle {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { inner: Mutex::new(HashMap::new()) })
    }

    pub fn check(&self, ip: IpAddr) -> Result<(), Duration> {
        let now = Instant::now();
        let mut map = self.inner.lock();
        if let Some(e) = map.get_mut(&ip) {
            if let Some(until) = e.locked_until {
                if until > now { return Err(until - now); }
                e.locked_until = None;
                e.failures.clear();
            }
        }
        Ok(())
    }

    pub fn record_failure(&self, ip: IpAddr) {
        let now = Instant::now();
        let mut map = self.inner.lock();
        let e = map.entry(ip).or_default();
        e.failures.retain(|t| now.duration_since(*t) <= WINDOW);
        e.failures.push(now);
        if e.failures.len() as u8 >= MAX_FAILURES {
            e.locked_until = Some(now + LOCKOUT);
        }
    }

    pub fn record_success(&self, ip: IpAddr) {
        let mut map = self.inner.lock();
        if let Some(e) = map.get_mut(&ip) {
            e.failures.clear();
            e.locked_until = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn five_failures_lock_out() {
        let t = LoginThrottle::new();
        let ip = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        for _ in 0..4 { t.check(ip).unwrap(); t.record_failure(ip); }
        // 5th failure trips the lockout.
        t.check(ip).unwrap();
        t.record_failure(ip);
        assert!(t.check(ip).is_err());
    }

    #[test]
    fn success_resets_failures() {
        let t = LoginThrottle::new();
        let ip = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        for _ in 0..4 { t.record_failure(ip); }
        t.record_success(ip);
        for _ in 0..4 { t.record_failure(ip); }
        assert!(t.check(ip).is_ok());
    }

    #[test]
    fn different_ips_isolated() {
        let t = LoginThrottle::new();
        let a = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        let b = IpAddr::V4(Ipv4Addr::new(5, 6, 7, 8));
        for _ in 0..6 { t.record_failure(a); }
        assert!(t.check(a).is_err());
        assert!(t.check(b).is_ok());
    }
}
