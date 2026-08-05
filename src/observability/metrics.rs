//! Minimal Prometheus exposition without a separate metrics runtime.

use std::sync::atomic::{AtomicU64, Ordering};

static REQUESTS_TOTAL: AtomicU64 = AtomicU64::new(0);
static RESPONSES_2XX: AtomicU64 = AtomicU64::new(0);
static RESPONSES_4XX: AtomicU64 = AtomicU64::new(0);
static RESPONSES_5XX: AtomicU64 = AtomicU64::new(0);

pub fn record(status: u16) {
    REQUESTS_TOTAL.fetch_add(1, Ordering::Relaxed);
    match status {
        200..=299 => {
            RESPONSES_2XX.fetch_add(1, Ordering::Relaxed);
        }
        400..=499 => {
            RESPONSES_4XX.fetch_add(1, Ordering::Relaxed);
        }
        500..=599 => {
            RESPONSES_5XX.fetch_add(1, Ordering::Relaxed);
        }
        _ => {}
    }
}

pub fn prometheus() -> String {
    format!(
        "# HELP webserver_requests_total HTTP requests completed by status class.\n# TYPE webserver_requests_total counter\nwebserver_requests_total {}\nwebserver_responses_total{{class=\"2xx\"}} {}\nwebserver_responses_total{{class=\"4xx\"}} {}\nwebserver_responses_total{{class=\"5xx\"}} {}\n",
        REQUESTS_TOTAL.load(Ordering::Relaxed),
        RESPONSES_2XX.load(Ordering::Relaxed),
        RESPONSES_4XX.load(Ordering::Relaxed),
        RESPONSES_5XX.load(Ordering::Relaxed),
    )
}
