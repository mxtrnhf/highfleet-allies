//! Pure battle TWR gate. A zero cutoff preserves pre-gate behavior.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_is_inclusive() {
        assert!(!allows(Some(2.99), 3.0));
        assert!(allows(Some(3.0), 3.0));
        assert!(allows(Some(3.01), 3.0));
    }

    #[test]
    fn invalid_or_missing_twr_fails_enabled_gate() {
        for twr in [None, Some(f32::NAN), Some(f32::INFINITY), Some(-1.0)] {
            assert!(!allows(twr, 3.0));
            assert!(allows(twr, 0.0));
        }
    }

    #[test]
    fn invalid_cutoff_disables_gate() {
        for cutoff in [-1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(sanitize_cutoff(cutoff), 0.0);
        }
        assert_eq!(sanitize_cutoff(3.0), 3.0);
    }
}
