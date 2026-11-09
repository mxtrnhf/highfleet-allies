//! Wall-clock helpers for UI timing and inexpensive, non-cryptographic sound/doctrine choices.
//! Battle simulation timing continues to use the native game clock.

/// A cheap pick in 0..n (no rand crate): the clock's nanoseconds. `n` must be nonzero.
pub(crate) fn pick(n: usize) -> usize {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos() as usize / 7)
        % n
}

/// Unix wall-clock milliseconds, or zero before the epoch. This is not a monotonic clock;
/// callers retain their existing wrapping elapsed-time checks.
pub(crate) fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
