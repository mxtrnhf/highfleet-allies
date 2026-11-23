//! Pure strategic/Shipworks TWR maneuver gate and hf_ai boost-strength limiter.

/// A finite, non-negative threshold; non-finite input becomes `0` (which disables the gate/limiter).
pub fn sanitize_cutoff(cutoff: f32) -> f32 {
    if cutoff.is_finite() {
        cutoff.max(0.0)
    } else {
        0.0
    }
}

/// Whether the TWR gate passes: a zero cutoff disables it, otherwise a finite TWR must reach the cutoff.
pub fn allows(twr: Option<f32>, cutoff: f32) -> bool {
    cutoff == 0.0 || twr.is_some_and(|twr| twr.is_finite() && twr >= cutoff)
}

/// The boost multiplier: `multiplier` strictly above `limit`, `1.0` otherwise; zero settings disable the limiter.
pub fn strength_multiplier(twr: Option<f32>, limit: f32, multiplier: f32) -> f32 {
    if limit == 0.0 || multiplier == 0.0 {
        return 1.0;
    }
    if twr.is_some_and(|twr| twr.is_finite() && twr > limit) {
        multiplier
    } else {
        1.0
    }
}
