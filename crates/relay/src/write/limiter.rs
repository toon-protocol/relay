//! A sliding-window rate limiter for the free ephemeral lane.
//!
//! With no payment in front of it, request volume is the only admission
//! control that lane has. It is a sliding-window log rather than a counter
//! per fixed window: a fixed window lets a caller burst twice the limit
//! across the boundary between two of them.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// At most `max_requests` per `window` for each key.
#[derive(Debug)]
pub(super) struct RateLimiter {
    max_requests: usize,
    window: Duration,
    hits: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl RateLimiter {
    pub(super) fn new(max_requests: u64, window: Duration) -> Self {
        Self {
            max_requests: usize::try_from(max_requests).unwrap_or(usize::MAX),
            window,
            hits: Mutex::default(),
        }
    }

    /// Whether `key` is under its limit, counting this call if it is. A call
    /// over the limit is not counted.
    pub(super) fn allow(&self, key: &str) -> bool {
        self.allow_at(key, Instant::now())
    }

    fn allow_at(&self, key: &str, now: Instant) -> bool {
        // A poisoned lock only means another request panicked mid-count; the
        // log is still a valid log.
        let mut hits = self
            .hits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let log = hits.entry(key.to_string()).or_default();
        while log
            .front()
            .is_some_and(|hit| now.saturating_duration_since(*hit) > self.window)
        {
            log.pop_front();
        }
        if log.len() >= self.max_requests {
            return false;
        }
        log.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_over_its_limit_is_refused_until_its_hits_age_out() {
        let limiter = RateLimiter::new(2, Duration::from_secs(10));
        let start = Instant::now();
        assert!(limiter.allow_at("a", start));
        assert!(limiter.allow_at("a", start + Duration::from_secs(1)));
        assert!(!limiter.allow_at("a", start + Duration::from_secs(2)));
        // The refused call was not counted: the first hit ages out at 10s.
        assert!(limiter.allow_at("a", start + Duration::from_secs(11)));
        assert!(!limiter.allow_at("a", start + Duration::from_secs(11)));
    }

    #[test]
    fn keys_do_not_share_a_budget() {
        let limiter = RateLimiter::new(1, Duration::from_secs(10));
        let now = Instant::now();
        assert!(limiter.allow_at("a", now));
        assert!(limiter.allow_at("b", now));
        assert!(!limiter.allow_at("a", now));
    }
}
