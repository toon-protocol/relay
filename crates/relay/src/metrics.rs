//! `GET /metrics`: the relay's telemetry, on the write port beside `/health`.
//!
//! The TypeScript relay's keys survive so a dashboard that reads them keeps
//! working (#185, story 44), minus one block. There is no event loop to
//! measure, so `eventLoopDelayMs` is gone. `verify` stays and reports the
//! native implementation: signatures are checked by libsecp256k1 on the
//! request's own task, which is the TypeScript relay's `workers: 0` path, so
//! `workers` is always 0.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::clock::unix_millis;
use crate::{Config, Relay};

/// How many recent verifications the percentiles are taken over.
const VERIFY_WINDOW: usize = 2048;

/// What verifies signatures: the C library `nostr` calls through `secp256k1`.
const VERIFY_IMPLEMENTATION: &str = "libsecp256k1-native";

/// The free ephemeral lane's bounds, as configured.
#[derive(Debug, Clone, Copy)]
struct EphemeralLane {
    max_requests: u32,
    window_ms: u64,
    max_body_bytes: u32,
}

#[derive(Debug, Default)]
struct VerifyTimings {
    count: u64,
    total_ms: f64,
    max_ms: f64,
    /// The last [`VERIFY_WINDOW`] durations, as a ring.
    window: Vec<f64>,
    cursor: usize,
}

/// The live registry behind `GET /metrics`. Cheap to clone: every clone is
/// the same registry.
#[derive(Debug, Clone)]
pub(crate) struct Metrics {
    lane: EphemeralLane,
    verify: Arc<Mutex<VerifyTimings>>,
}

impl Metrics {
    pub(crate) fn new(config: &Config) -> Self {
        Self {
            lane: EphemeralLane {
                max_requests: config.ephemeral_rate_limit,
                window_ms: config.ephemeral_rate_window_ms,
                max_body_bytes: config.ephemeral_max_body_bytes,
            },
            verify: Arc::default(),
        }
    }

    /// Record one signature verification and how long it took.
    pub(crate) fn record_verify(&self, took: Duration) {
        let ms = took.as_secs_f64() * 1000.0;
        let mut timings = self.timings();
        timings.count += 1;
        timings.total_ms += ms;
        timings.max_ms = timings.max_ms.max(ms);
        if timings.window.len() < VERIFY_WINDOW {
            timings.window.push(ms);
        } else {
            let at = timings.cursor;
            timings.window[at] = ms;
        }
        timings.cursor = (timings.cursor + 1) % VERIFY_WINDOW;
    }

    /// A panic while the lock was held cannot leave the counters half
    /// updated in a way that matters to a dashboard, so a poisoned lock is
    /// used as it is.
    fn timings(&self) -> MutexGuard<'_, VerifyTimings> {
        self.verify
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn snapshot(&self) -> Snapshot {
        let timings = self.timings();
        let mut recent = timings.window.clone();
        recent.sort_by(f64::total_cmp);
        let mean_ms = if timings.count == 0 {
            0.0
        } else {
            // A count past 2^53 is not a count anyone reads a mean from.
            timings.total_ms / timings.count as f64
        };
        Snapshot {
            timestamp: unix_millis(),
            verify: Verify {
                implementation: VERIFY_IMPLEMENTATION,
                workers: 0,
                count: timings.count,
                mean_ms: round(mean_ms),
                max_ms: round(timings.max_ms),
                p50_ms: round(percentile(&recent, 0.5)),
                p99_ms: round(percentile(&recent, 0.99)),
            },
            ephemeral_write_lane: Lane {
                enabled: true,
                rate_limit: RateLimit {
                    max_requests: self.lane.max_requests,
                    window_ms: self.lane.window_ms,
                },
                max_body_bytes: self.lane.max_body_bytes,
            },
        }
    }
}

/// The value at `fraction` of the way through `sorted`, nearest rank.
fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (fraction * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

/// Microsecond precision, as the TypeScript relay rounds.
fn round(ms: f64) -> f64 {
    (ms * 1000.0).round() / 1000.0
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    /// Milliseconds since the Unix epoch.
    timestamp: u64,
    verify: Verify,
    ephemeral_write_lane: Lane,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Verify {
    implementation: &'static str,
    workers: u32,
    count: u64,
    mean_ms: f64,
    max_ms: f64,
    p50_ms: f64,
    p99_ms: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Lane {
    enabled: bool,
    rate_limit: RateLimit,
    max_body_bytes: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RateLimit {
    max_requests: u32,
    window_ms: u64,
}

pub(crate) async fn metrics(State(relay): State<Relay>) -> Json<Snapshot> {
    Json(relay.metrics.snapshot())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Metrics {
        let config = Config::from_env(|name| (name == "TOON_SECRET_KEY").then(|| "1".repeat(64)))
            .expect("a secret key is a complete configuration");
        Metrics::new(&config)
    }

    #[test]
    fn nothing_verified_reports_zeroes() {
        let verify = registry().snapshot().verify;
        assert_eq!((verify.count, verify.max_ms, verify.p99_ms), (0, 0.0, 0.0));
    }

    #[test]
    fn verifications_are_counted_and_summarised() {
        let metrics = registry();
        for ms in [1, 2, 3, 4] {
            metrics.record_verify(Duration::from_millis(ms));
        }
        let verify = metrics.snapshot().verify;
        assert_eq!(verify.count, 4);
        assert_eq!(verify.max_ms, 4.0);
        assert_eq!(verify.mean_ms, 2.5);
        assert_eq!(verify.p50_ms, 2.0);
        assert_eq!(verify.p99_ms, 4.0);
    }

    #[test]
    fn percentiles_cover_only_the_recent_window_but_the_max_covers_all() {
        let metrics = registry();
        metrics.record_verify(Duration::from_millis(500));
        for _ in 0..VERIFY_WINDOW {
            metrics.record_verify(Duration::from_millis(1));
        }
        let verify = metrics.snapshot().verify;
        assert_eq!(verify.max_ms, 500.0);
        assert_eq!(verify.p99_ms, 1.0);
    }
}
