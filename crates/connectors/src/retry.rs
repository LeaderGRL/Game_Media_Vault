//! Bounded retries of transient Source failures.

use std::{
    collections::hash_map::RandomState,
    hash::{BuildHasher, Hasher},
    time::{Duration, SystemTime},
};

/// How a transport retries transient failures: each retry waits for a backoff that doubles from
/// `base_delay` up to `max_delay`, jittered so Sources failing together do not retry together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Attempts in all, including the first; 1 never retries.
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(8),
        }
    }
}

impl RetryPolicy {
    /// How long to wait before retry number `retry` (1 for the first), at least what the Source
    /// asked for in `retry_after` within `max_delay`.
    pub(crate) fn delay_before_retry(&self, retry: u32, retry_after: Option<Duration>) -> Duration {
        self.jittered_delay(retry, retry_after, random_fraction())
    }

    /// The backoff of `retry`, between half its ceiling and the ceiling as `fraction` goes from
    /// 0 to 1.
    fn jittered_delay(&self, retry: u32, retry_after: Option<Duration>, fraction: f64) -> Duration {
        let doublings = retry.saturating_sub(1).min(31);
        let ceiling = self
            .base_delay
            .saturating_mul(1_u32 << doublings)
            .min(self.max_delay);
        let jittered = ceiling / 2 + ceiling.mul_f64(fraction.clamp(0.0, 1.0) / 2.0);
        let requested = retry_after.unwrap_or_default().min(self.max_delay);
        jittered.max(requested)
    }
}

/// A fraction in [0, 1) that differs between calls; precise randomness is not needed.
fn random_fraction() -> f64 {
    let mut hasher = RandomState::new().build_hasher();
    if let Ok(elapsed) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
        hasher.write_u128(elapsed.as_nanos());
    }
    (hasher.finish() >> 11) as f64 / (1_u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLICY: RetryPolicy = RetryPolicy {
        max_attempts: 5,
        base_delay: Duration::from_millis(100),
        max_delay: Duration::from_millis(500),
    };

    #[test]
    fn backoff_doubles_from_the_base_delay_up_to_the_maximum() {
        let ceilings: Vec<Duration> = (1..=5)
            .map(|retry| POLICY.jittered_delay(retry, None, 1.0))
            .collect();

        assert_eq!(
            ceilings,
            [100, 200, 400, 500, 500].map(Duration::from_millis)
        );
    }

    #[test]
    fn jitter_keeps_each_backoff_between_half_its_ceiling_and_the_ceiling() {
        assert_eq!(
            POLICY.jittered_delay(2, None, 0.0),
            Duration::from_millis(100)
        );
        assert_eq!(
            POLICY.jittered_delay(2, None, 0.5),
            Duration::from_millis(150)
        );
        for _ in 0..100 {
            let delay = POLICY.delay_before_retry(3, None);
            assert!(
                (Duration::from_millis(200)..=Duration::from_millis(400)).contains(&delay),
                "{delay:?}"
            );
        }
    }

    #[test]
    fn a_requested_wait_is_honoured_within_the_maximum() {
        assert_eq!(
            POLICY.jittered_delay(1, Some(Duration::from_millis(300)), 0.0),
            Duration::from_millis(300)
        );
        assert_eq!(
            POLICY.jittered_delay(1, Some(Duration::from_secs(60)), 0.0),
            Duration::from_millis(500)
        );
    }
}
