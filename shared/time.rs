//! Wall-clock helpers for UI timing and inexpensive, non-cryptographic sound/doctrine choices.
//! Battle simulation timing continues to use the native game clock.

use std::time::{SystemTime, UNIX_EPOCH};

/// A cheap pick in `0..choices` based on the clock's nanoseconds. `choices` must be nonzero.
pub fn pick(choices: usize) -> usize {
    let sample = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.subsec_nanos() as usize / 7);
    sample % choices
}

/// Unix wall-clock milliseconds, or zero before the epoch. This is not a monotonic clock;
/// callers retain their existing wrapping elapsed-time checks.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
