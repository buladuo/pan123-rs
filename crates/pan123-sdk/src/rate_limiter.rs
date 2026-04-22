use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<RateLimiterInner>>,
}

#[derive(Debug)]
struct RateLimiterInner {
    tokens: f64,
    capacity: f64,
    refill_rate: f64,
    last_refill: Instant,
}

impl RateLimiter {
    pub fn new(requests_per_second: f64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RateLimiterInner {
                tokens: requests_per_second,
                capacity: requests_per_second,
                refill_rate: requests_per_second,
                last_refill: Instant::now(),
            })),
        }
    }

    pub fn with_burst(requests_per_second: f64, burst_capacity: f64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RateLimiterInner {
                tokens: burst_capacity,
                capacity: burst_capacity,
                refill_rate: requests_per_second,
                last_refill: Instant::now(),
            })),
        }
    }

    pub fn acquire(&self) {
        self.acquire_n(1.0);
    }

    pub fn acquire_n(&self, tokens: f64) {
        loop {
            let mut inner = self.inner.lock().expect("rate limiter lock poisoned");
            inner.refill();

            if inner.tokens >= tokens {
                inner.tokens -= tokens;
                return;
            }

            let deficit = tokens - inner.tokens;
            let wait_time = Duration::from_secs_f64(deficit / inner.refill_rate);
            drop(inner);

            std::thread::sleep(wait_time.min(Duration::from_millis(100)));
        }
    }

    pub fn try_acquire(&self) -> bool {
        self.try_acquire_n(1.0)
    }

    pub fn try_acquire_n(&self, tokens: f64) -> bool {
        let mut inner = self.inner.lock().expect("rate limiter lock poisoned");
        inner.refill();

        if inner.tokens >= tokens {
            inner.tokens -= tokens;
            true
        } else {
            false
        }
    }

    pub fn available_tokens(&self) -> f64 {
        let mut inner = self.inner.lock().expect("rate limiter lock poisoned");
        inner.refill();
        inner.tokens
    }
}

impl RateLimiterInner {
    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        let new_tokens = elapsed * self.refill_rate;

        self.tokens = (self.tokens + new_tokens).min(self.capacity);
        self.last_refill = now;
    }
}

#[derive(Debug, Clone)]
pub struct RateLimiterConfig {
    pub api_requests_per_second: f64,
    pub upload_bytes_per_second: Option<u64>,
    pub download_bytes_per_second: Option<u64>,
}

impl Default for RateLimiterConfig {
    fn default() -> Self {
        Self {
            api_requests_per_second: 10.0,
            upload_bytes_per_second: None,
            download_bytes_per_second: None,
        }
    }
}

impl RateLimiterConfig {
    pub fn conservative() -> Self {
        Self {
            api_requests_per_second: 5.0,
            upload_bytes_per_second: Some(5 * 1024 * 1024),
            download_bytes_per_second: Some(10 * 1024 * 1024),
        }
    }

    pub fn aggressive() -> Self {
        Self {
            api_requests_per_second: 20.0,
            upload_bytes_per_second: None,
            download_bytes_per_second: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn test_rate_limiter_basic() {
        let limiter = RateLimiter::new(10.0);

        for _ in 0..10 {
            assert!(limiter.try_acquire());
        }

        assert!(!limiter.try_acquire());
    }

    #[test]
    fn test_rate_limiter_refill() {
        let limiter = RateLimiter::new(10.0);

        for _ in 0..10 {
            limiter.acquire();
        }

        std::thread::sleep(Duration::from_millis(200));

        assert!(limiter.try_acquire());
    }

    #[test]
    fn test_rate_limiter_timing() {
        let limiter = RateLimiter::new(5.0);

        let start = Instant::now();
        for _ in 0..10 {
            limiter.acquire();
        }
        let elapsed = start.elapsed();

        assert!(elapsed >= Duration::from_secs(1));
        assert!(elapsed < Duration::from_millis(1500));
    }
}
