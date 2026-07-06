//! Retry policy (Tech Spec §4.3, S-3). This module owns the *policy* — how
//! long to wait before which attempt — as pure, testable data. The retry
//! *loop* lives in the engine (`emberly-core`), because it owns the event
//! channel and must surface every retry (Design §6.1, "never silent") and
//! because a mid-stream drop is retried as a whole turn, not a single request.
//!
//! Backoff is exponential with **full jitter** (random in `[0, cap]`), which
//! avoids synchronized retry storms.

use std::time::Duration;

/// How many times and how patiently to retry a retryable failure.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Total attempts including the first (so `3` = up to 2 retries).
    pub max_attempts: u32,
    /// Base delay; attempt `n` waits up to `base * 2^(n-1)`, capped.
    pub base_delay: Duration,
    /// Upper bound on any single wait.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(8),
        }
    }
}

impl RetryPolicy {
    /// The delay before retry number `retry` (1 = first retry), with full
    /// jitter. `retry` of 0 is treated as 1.
    #[must_use]
    pub fn delay_for(&self, retry: u32) -> Duration {
        let shift = retry.saturating_sub(1).min(31);
        let factor = 1u32 << shift;
        let capped = self.base_delay.saturating_mul(factor).min(self.max_delay);
        let cap_millis = u64::try_from(capped.as_millis()).unwrap_or(u64::MAX);
        if cap_millis == 0 {
            Duration::ZERO
        } else {
            Duration::from_millis(fastrand::u64(0..=cap_millis))
        }
    }

    /// Whether another attempt is allowed after `attempts_so_far` attempts.
    #[must_use]
    pub fn may_retry(&self, attempts_so_far: u32) -> bool {
        attempts_so_far < self.max_attempts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_never_exceeds_the_cap() {
        let policy = RetryPolicy {
            max_attempts: 5,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(2),
        };
        for retry in 1..=10u32 {
            for _ in 0..50 {
                assert!(policy.delay_for(retry) <= policy.max_delay);
            }
        }
    }

    #[test]
    fn early_retries_stay_within_exponential_bound() {
        let policy = RetryPolicy {
            max_attempts: 5,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(60),
        };
        // Retry 1: <= 100ms; retry 2: <= 200ms; retry 3: <= 400ms.
        for _ in 0..50 {
            assert!(policy.delay_for(1) <= Duration::from_millis(100));
            assert!(policy.delay_for(2) <= Duration::from_millis(200));
            assert!(policy.delay_for(3) <= Duration::from_millis(400));
        }
    }

    #[test]
    fn may_retry_respects_max_attempts() {
        let policy = RetryPolicy::default(); // 3 attempts
        assert!(policy.may_retry(1));
        assert!(policy.may_retry(2));
        assert!(!policy.may_retry(3));
    }
}
