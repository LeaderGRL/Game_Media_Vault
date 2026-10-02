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

/// The wait a `Retry-After` header asks for, given in seconds or as an HTTP date (the IMF-fixdate
/// form, `Sun, 06 Nov 1994 08:49:37 GMT`); a date already past asks for none.
pub(crate) fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let date = http_date(value)?;
    Some(date.duration_since(now).unwrap_or_default())
}

/// Reads an IMF-fixdate, the HTTP date form servers must send.
fn http_date(value: &str) -> Option<SystemTime> {
    let (_, rest) = value.split_once(", ")?;
    let mut parts = rest.split(' ');
    let day: u32 = parts.next()?.parse().ok()?;
    let month = match parts.next()? {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year: i64 = parts.next()?.parse().ok()?;
    let mut clock = parts.next()?.split(':');
    let hours: u64 = clock.next()?.parse().ok()?;
    let minutes: u64 = clock.next()?.parse().ok()?;
    let seconds: u64 = clock.next()?.parse().ok()?;
    if parts.next()? != "GMT" || parts.next().is_some() || !(1..=31).contains(&day) {
        return None;
    }
    if hours > 23 || minutes > 59 || seconds > 60 {
        return None;
    }
    let days = u64::try_from(days_from_civil(year, month, day)).ok()?;
    let since_epoch = days * 86_400 + hours * 3_600 + minutes * 60 + seconds;
    SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(since_epoch))
}

/// Days from 1970-01-01 to a date of the proleptic Gregorian calendar.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
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
    fn retry_after_is_read_in_seconds_or_as_an_http_date() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(784_111_740);

        assert_eq!(
            parse_retry_after("120", now),
            Some(Duration::from_secs(120))
        );
        // 1994-11-06 08:49:37 GMT is 37 seconds after `now`.
        assert_eq!(
            parse_retry_after("Sun, 06 Nov 1994 08:49:37 GMT", now),
            Some(Duration::from_secs(37))
        );
        assert_eq!(
            parse_retry_after("Sun, 06 Nov 1994 08:48:00 GMT", now),
            Some(Duration::ZERO)
        );
        assert_eq!(parse_retry_after("soon", now), None);
        assert_eq!(
            parse_retry_after("Sun, 06 Nov 1994 08:49:37 PST", now),
            None
        );
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
