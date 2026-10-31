//! Pure strategic/Shipworks TWR gate. A zero cutoff preserves pre-gate behavior.
pub fn sanitize_cutoff(cutoff: f32) -> f32 {
    if cutoff.is_finite() {
        cutoff.max(0.0)
    } else {
        0.0
    }
}

pub fn allows(twr: Option<f32>, cutoff: f32) -> bool {
    cutoff == 0.0 || twr.is_some_and(|twr| twr.is_finite() && twr >= cutoff)
}
