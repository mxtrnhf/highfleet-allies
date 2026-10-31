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
