//! Lightweight per-client request limiter.

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

struct Bucket {
    started: Instant,
    requests: u32,
}

const MAX_CLIENT_BUCKETS: usize = 65_536;

struct RateLimiter {
    buckets: HashMap<IpAddr, Bucket>,
    last_pruned: Instant,
}

fn limiter() -> &'static Mutex<RateLimiter> {
    static LIMITER: OnceLock<Mutex<RateLimiter>> = OnceLock::new();
    LIMITER.get_or_init(|| {
        Mutex::new(RateLimiter {
            buckets: HashMap::new(),
            last_pruned: Instant::now(),
        })
    })
}

pub fn allow(client: IpAddr, per_minute: u32) -> bool {
    if per_minute == 0 {
        return true;
    }
    let now = Instant::now();
    let mut limiter = limiter().lock().expect("rate limiter lock");
    if now.duration_since(limiter.last_pruned) >= Duration::from_secs(60) {
        limiter
            .buckets
            .retain(|_, bucket| now.duration_since(bucket.started) < Duration::from_secs(120));
        limiter.last_pruned = now;
    }
    if limiter.buckets.len() >= MAX_CLIENT_BUCKETS && !limiter.buckets.contains_key(&client) {
        return false;
    }
    let bucket = limiter.buckets.entry(client).or_insert(Bucket {
        started: now,
        requests: 0,
    });
    if now.duration_since(bucket.started) >= Duration::from_secs(60) {
        bucket.started = now;
        bucket.requests = 0;
    }
    bucket.requests = bucket.requests.saturating_add(1);
    bucket.requests <= per_minute
}
