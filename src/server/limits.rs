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

fn buckets() -> &'static Mutex<HashMap<IpAddr, Bucket>> {
    static BUCKETS: OnceLock<Mutex<HashMap<IpAddr, Bucket>>> = OnceLock::new();
    BUCKETS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn allow(client: IpAddr, per_minute: u32) -> bool {
    if per_minute == 0 {
        return true;
    }
    let now = Instant::now();
    let mut buckets = buckets().lock().expect("rate limiter lock");
    buckets.retain(|_, bucket| now.duration_since(bucket.started) < Duration::from_secs(120));
    let bucket = buckets.entry(client).or_insert(Bucket {
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
