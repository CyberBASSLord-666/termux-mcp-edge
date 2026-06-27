//! Lightweight runtime smoke tests.

use std::time::{Duration, Instant};

#[tokio::test]
async fn async_runtime_timer_advances_for_latency_sensitive_tests() {
    let start = Instant::now();
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(start.elapsed() >= Duration::from_millis(10));
}
