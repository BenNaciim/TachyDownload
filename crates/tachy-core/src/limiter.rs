//! Shared bandwidth limiter (`tokenBucket.js`). `rate` is in bytes per second, 0 means unlimited. A caller may
//! take more than the bucket holds: the balance goes negative and the caller waits until it has been repaid,
//! which keeps the long-run rate exact.

use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use crate::errors::{Fail, Result};

struct Bucket {
    rate: f64,
    tokens: f64,
    last: Instant,
}

pub struct TokenBucket {
    inner: Mutex<Bucket>,
}

impl TokenBucket {
    pub fn new(rate: f64) -> Self {
        let b = Self {
            inner: Mutex::new(Bucket {
                rate: 0.0,
                tokens: 0.0,
                last: Instant::now(),
            }),
        };
        b.set_rate(rate);
        b
    }

    /// Setting a rate refills the bucket, so callers set it only when it changed.
    pub fn set_rate(&self, rate: f64) {
        let mut b = self.inner.lock().unwrap();
        b.rate = if rate.is_finite() { rate.max(0.0) } else { 0.0 };
        b.tokens = b.rate;
        b.last = Instant::now();
    }

    pub fn rate(&self) -> f64 {
        self.inner.lock().unwrap().rate
    }

    /// Takes `bytes` and returns how long the caller must wait before using them.
    fn reserve(&self, bytes: usize) -> Duration {
        let mut b = self.inner.lock().unwrap();
        if b.rate <= 0.0 {
            return Duration::ZERO;
        }
        let now = Instant::now();
        let elapsed = now.duration_since(b.last).as_secs_f64();
        b.tokens = (b.tokens + elapsed * b.rate).min(b.rate);
        b.last = now;
        b.tokens -= bytes as f64;
        if b.tokens < 0.0 {
            Duration::from_secs_f64(-b.tokens / b.rate)
        } else {
            Duration::ZERO
        }
    }

    pub async fn take(&self, bytes: usize, cancel: &CancellationToken) -> Result<()> {
        let wait = self.reserve(bytes);
        if wait.is_zero() {
            return Ok(());
        }
        tokio::select! {
            _ = tokio::time::sleep(wait) => Ok(()),
            _ = cancel.cancelled() => Err(Fail::Cancelled),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unlimited_never_waits() {
        let b = TokenBucket::new(0.0);
        let t = Instant::now();
        b.take(10_000_000, &CancellationToken::new()).await.unwrap();
        assert!(t.elapsed() < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn the_balance_may_go_negative_and_is_repaid() {
        let b = TokenBucket::new(100_000.0);
        let c = CancellationToken::new();
        let t = Instant::now();
        b.take(100_000, &c).await.unwrap(); // the burst: free
        assert!(t.elapsed() < Duration::from_millis(100));
        b.take(50_000, &c).await.unwrap(); // owes 0.5 s
        let spent = t.elapsed().as_secs_f64();
        assert!((0.4..0.9).contains(&spent), "waited {spent}s");
    }

    #[tokio::test]
    async fn waiting_can_be_cancelled() {
        let b = TokenBucket::new(1000.0);
        let c = CancellationToken::new();
        b.take(1000, &c).await.unwrap();
        let c2 = c.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            c2.cancel();
        });
        assert!(matches!(b.take(100_000, &c).await, Err(Fail::Cancelled)));
    }

    #[test]
    fn set_rate_reports_the_rate() {
        let b = TokenBucket::new(5.0);
        assert_eq!(b.rate(), 5.0);
        b.set_rate(-3.0);
        assert_eq!(b.rate(), 0.0);
    }
}
