//! Per-project token bucket rate limiter (in-memory).
//!
//! Capacity = `max_per_minute`; refill rate = `max_per_minute / 60` tokens per second.
//! A request consumes one token; if none available, it is rejected and `retry_after` (seconds)
//! is returned for the `Retry-After` header.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

#[derive(Debug)]
pub struct RateLimiter {
    capacity: f64,
    refill_per_sec: f64,
    buckets: Mutex<HashMap<i64, Bucket>>,
}

#[derive(Debug, Clone, Copy)]
pub struct Decision {
    pub allowed: bool,
    /// Seconds the client should wait before retrying. Always 1 when refused (token bucket
    /// granularity is sub-second but Retry-After is whole seconds).
    pub retry_after: u32,
}

impl RateLimiter {
    pub fn new(max_per_minute: u32) -> Self {
        let cap = f64::from(max_per_minute.max(1));
        Self {
            capacity: cap,
            refill_per_sec: cap / 60.0,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    pub fn check(&self, project_id: i64) -> Decision {
        let now = Instant::now();
        let mut guard = match self.buckets.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let bucket = guard.entry(project_id).or_insert_with(|| Bucket {
            tokens: self.capacity,
            last_refill: now,
        });

        let elapsed = now.duration_since(bucket.last_refill);
        bucket.tokens =
            (bucket.tokens + elapsed.as_secs_f64() * self.refill_per_sec).min(self.capacity);
        bucket.last_refill = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Decision {
                allowed: true,
                retry_after: 0,
            }
        } else {
            Decision {
                allowed: false,
                retry_after: 1,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn allows_up_to_capacity() {
        let rl = RateLimiter::new(3);
        assert!(rl.check(1).allowed);
        assert!(rl.check(1).allowed);
        assert!(rl.check(1).allowed);
        assert!(!rl.check(1).allowed);
    }

    #[test]
    fn separate_buckets_per_project() {
        let rl = RateLimiter::new(1);
        assert!(rl.check(1).allowed);
        assert!(!rl.check(1).allowed);
        assert!(rl.check(2).allowed);
    }

    #[test]
    fn refills_over_time() {
        // 120/min => 2/sec
        let rl = RateLimiter::new(120);
        for _ in 0..120 {
            assert!(rl.check(7).allowed);
        }
        assert!(!rl.check(7).allowed);
        sleep(Duration::from_millis(600));
        // At least 1 token should be refilled in 600ms.
        assert!(rl.check(7).allowed);
    }
}
