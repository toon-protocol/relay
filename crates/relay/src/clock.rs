//! The wall clock, as the relay's wire and its database carry it: whole
//! seconds for `storedAt` and the `received_at` column, milliseconds for
//! `/health`. One place, so a clock set before 1970 reads as 0 everywhere
//! rather than failing a liveness probe or a write.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn since_epoch() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

/// Seconds since the Unix epoch.
pub(crate) fn unix_seconds() -> u64 {
    since_epoch().as_secs()
}

/// Milliseconds since the Unix epoch.
pub(crate) fn unix_millis() -> u64 {
    u64::try_from(since_epoch().as_millis()).unwrap_or(u64::MAX)
}
