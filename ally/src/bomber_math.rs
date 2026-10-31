//! Ship bombing-run decisions. World Y points down.

pub const HULL_MARGIN: f32 = 80.0;
/// Release ceiling above the target. An upper bound only: a ship that spawns below it
/// needs no special handling and simply builds speed and releases.
pub const RELEASE_HEIGHT: f32 = 1000.0;
/// Command/attitude time assumed before recovery thrust starts arresting descent.
const PULL_OUT_RESPONSE_S: f32 = 0.75;
/// Minimum time spent pulling up before normal AI resumes.
pub const PULL_OUT_S: f64 = 1.5;
/// Descent is limited to this share of the speed the current height can still recover from.
const SAFE_DESCENT_FRACTION: f32 = 0.85;
/// Fastest dive the governor will ask for.
const MAX_DESCENT_SPEED: f32 = 220.0;
/// Horizontal distance from the target X beyond which the dive holds altitude to line up.
const LATERAL_ALIGN_DISTANCE: f32 = 220.0;
/// Lateral steering magnitude of the dive waypoint.
const DIVE_LATERAL: f32 = 160.0;
/// Smallest non-zero downward component: a zero-length request means "thrust up" natively.
const MIN_DIVE_COMPONENT: f32 = 40.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Dive,
    Salvo,
    Recover,
    Done,
}

pub struct Run {
    pub phase: Phase,
    pub recover_until: f64,
    pub direction: f32,
    pub started: f64,
    /// The run already began below the release ceiling, so there is no room for a lateral
    /// altitude-hold: dive and release instead of hovering to line up.
    pub low_start: bool,
}

pub struct Flight {
    pub now: f64,
    pub position: (f32, f32),
    pub target: (f32, f32),
    pub velocity: (f32, f32),
    pub own_radius: f32,
    pub target_radius: f32,
    pub recovery_acceleration: f32,
}

impl Run {
    pub fn new(flight: &Flight) -> Self {
        Self {
            phase: Phase::Dive,
            recover_until: 0.0,
            direction: if flight.target.0 >= flight.position.0 {
                1.0
            } else {
                -1.0
            },
            started: flight.now,
            low_start: (flight.target.1 - flight.position.1) <= RELEASE_HEIGHT,
        }
    }

    pub fn recover(&mut self, now: f64) {
        self.phase = Phase::Recover;
        self.recover_until = now + PULL_OUT_S;
    }

    /// Descent speed that still leaves room to arrest the dive at this height.
    pub fn safe_descent(height_above_clearance: f32, recovery_acceleration: f32) -> f32 {
        if height_above_clearance <= 0.0 || recovery_acceleration <= 0.0 {
            return 0.0;
        }
        // Solve v^2 / (2a) + v * response = budget for the positive root.
        let response = PULL_OUT_RESPONSE_S;
        recovery_acceleration
            * ((response * response + 2.0 * height_above_clearance / recovery_acceleration).sqrt()
                - response)
    }

    pub fn waypoint(&mut self, flight: &Flight) -> (f32, f32) {
        let (x, y) = flight.position;
        let clearance = flight.own_radius + flight.target_radius + HULL_MARGIN;
        let height = flight.target.1 - y;

        if self.phase == Phase::Recover
            && flight.now >= self.recover_until
            && flight.velocity.1 <= 0.0
            && height > clearance
        {
            self.phase = Phase::Done;
        }

        match self.phase {
            Phase::Done => flight.position,
            Phase::Recover => (x + self.direction * 200.0, y - 200.0),
            Phase::Dive | Phase::Salvo => {
                // Emergency pull-out only when the current descent is genuinely unrecoverable.
                let downward_speed = flight.velocity.1.max(0.0);
                let pull_out_distance = downward_speed * downward_speed
                    / (2.0 * flight.recovery_acceleration)
                    + downward_speed * PULL_OUT_RESPONSE_S;
                if height <= clearance + pull_out_distance {
                    self.recover(flight.now);
                    return (x + self.direction * 200.0, y - 200.0);
                }

                // Continuous downward thrust whose speed is capped by the height available to
                // recover. Without this the ship reaches ~250 u/s, the pull-out distance exceeds
                // the remaining height, and the run aborts before the aim can converge.
                let budget = (height - clearance).max(0.0);
                let safe_descent = (Self::safe_descent(budget, flight.recovery_acceleration)
                    * SAFE_DESCENT_FRACTION)
                    .min(MAX_DESCENT_SPEED);
                let vertical = ((safe_descent - downward_speed) * 3.0).clamp(-200.0, 200.0);

                // Far from the target's X: hold altitude so lateral aim has time to line up.
                // Never gate away braking: a negative (upward) component means too fast.
                let dx = flight.target.0 - x;
                let alignment_gate = if self.low_start {
                    0.0
                } else {
                    ((dx.abs() - LATERAL_ALIGN_DISTANCE) / LATERAL_ALIGN_DISTANCE).clamp(0.0, 1.0)
                };
                let mut vertical = if vertical > 0.0 {
                    vertical * (1.0 - alignment_gate)
                } else {
                    vertical
                };
                if vertical.abs() < MIN_DIVE_COMPONENT {
                    vertical = MIN_DIVE_COMPONENT;
                }

                (x + dx.clamp(-DIVE_LATERAL, DIVE_LATERAL), y + vertical)
            }
        }
    }
}
