//! Counts successful tactical carrier launches; the stock AI owns selection and timing.

pub const DEFAULT_GROUP_SIZE: i32 = 3;
pub const DEFAULT_DELAY: f32 = 0.5;
pub const STOCK_COOLDOWN: f32 = 6.0;

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub size: u32,
    pub delay: f32,
}

impl Config {
    pub fn sanitized(size: i32, delay: f32) -> Self {
        Self {
            size: size.clamp(1, 64) as u32,
            delay: if delay.is_finite() {
                delay.clamp(0.05, 60.0)
            } else {
                DEFAULT_DELAY
            },
        }
    }
}

#[derive(Default, Debug)]
pub struct Group {
    remaining: u32,
    last_launch: Option<f64>,
    pending_cooldown: Option<f32>,
}

impl Group {
    pub fn launched(&mut self, now: f64, config: Config) {
        let continuing = self.remaining > 0
            && self
                .last_launch
                .is_some_and(|last| now >= last && now - last <= f64::from(config.delay) + 1.0);
        self.remaining = if continuing {
            self.remaining - 1
        } else {
            config.size - 1
        };
        self.last_launch = Some(now);
        self.pending_cooldown = Some(if self.remaining > 0 {
            config.delay
        } else {
            STOCK_COOLDOWN
        });
    }

    /// A failed stock attempt cancels a partial group. The game subtracts this frame's
    /// delta immediately after the cooldown store, so compensate only the group delay.
    pub fn finish_attempt(&mut self, frame_delta: f32) -> f32 {
        match self.pending_cooldown.take() {
            Some(delay) if self.remaining > 0 => delay + frame_delta.max(0.0),
            Some(delay) => delay,
            None => {
                *self = Self::default();
                STOCK_COOLDOWN
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_launches_are_spaced_then_stock_cooldown_returns() {
        let config = Config::sanitized(3, 0.5);
        let mut group = Group::default();
        for now in [10.0, 10.5] {
            group.launched(now, config);
            assert!((group.finish_attempt(0.02) - 0.02 - 0.5).abs() < 1.0e-6);
        }
        group.launched(11.0, config);
        assert_eq!(group.finish_attempt(0.02), STOCK_COOLDOWN);
        group.launched(17.0, config);
        assert_eq!(group.finish_attempt(0.0), 0.5);
    }

    #[test]
    fn one_plane_preserves_stock_cadence() {
        let mut group = Group::default();
        for now in [0.0, 6.0, 12.0] {
            group.launched(now, Config::sanitized(1, 0.5));
            assert_eq!(group.finish_attempt(0.02), STOCK_COOLDOWN);
        }
    }

    #[test]
    fn failed_attempt_cancels_partial_group() {
        let mut group = Group::default();
        let config = Config::sanitized(3, 0.5);
        group.launched(0.0, config);
        assert_eq!(group.finish_attempt(0.0), 0.5);
        assert_eq!(group.finish_attempt(0.0), STOCK_COOLDOWN);
        group.launched(6.5, config);
        assert_eq!(group.finish_attempt(0.0), 0.5);
    }

    #[test]
    fn clock_reset_and_long_interruptions_start_fresh_groups() {
        let config = Config::sanitized(3, 0.5);
        for next in [0.0, 30.0] {
            let mut group = Group::default();
            group.launched(10.0, config);
            group.finish_attempt(0.0);
            group.launched(next, config);
            group.finish_attempt(0.0);
            group.launched(next + 0.5, config);
            assert_eq!(group.finish_attempt(0.0), 0.5);
        }
    }

    #[test]
    fn carriers_have_independent_group_counts_and_custom_delays() {
        let config = Config::sanitized(2, 1.25);
        let mut first = Group::default();
        let mut second = Group::default();
        first.launched(0.0, config);
        second.launched(0.1, config);
        assert_eq!(first.finish_attempt(0.0), 1.25);
        assert_eq!(second.finish_attempt(0.0), 1.25);
        first.launched(1.25, config);
        assert_eq!(first.finish_attempt(0.0), STOCK_COOLDOWN);
    }

    #[test]
    fn config_limits_prevent_zero_groups_and_instant_launches() {
        for size in [-3, 0, 1] {
            assert_eq!(Config::sanitized(size, 0.5).size, 1);
        }
        assert_eq!(Config::sanitized(i32::MAX, 0.5).size, 64);
        for delay in [-1.0, 0.0] {
            assert_eq!(Config::sanitized(3, delay).delay, 0.05);
        }
        for delay in [f32::NAN, f32::INFINITY] {
            assert_eq!(Config::sanitized(3, delay).delay, DEFAULT_DELAY);
        }
        assert_eq!(Config::sanitized(3, 1000.0).delay, 60.0);
    }
}
