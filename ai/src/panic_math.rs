//! The pure part of the ported qol `ai_flares`: direction math and the per-ship panic timers. No game memory, no
//! dependencies, so it can be tested on the host (`rustc --edition 2021 --test src/panic_math.rs`, see README).
//! The function bodies are the original `hf_qol_fork/src/ai_flares.rs` code, moved here unchanged except where
//! noted; the hook glue that reads game memory stays in `ai_flares.rs`.

/// A still-coming missile keeps the evasion going for at least this many more seconds.
pub const EVASION_EXTENSION: f64 = 1.0;
/// The same for a panic evasion.
pub const PANIC_EVASION_EXTENSION: f64 = 2.5;
/// Share of retreat mixed into the sidestep (1.0 would be as much retreat as sidestep).
pub const EVASION_RETREAT: f32 = 0.5;
/// Waypoint distance used when the ship has no usable course (the AI normally uses 200).
pub const EVASION_FALLBACK_LENGTH: f32 = 200.0;
/// Engine commands weaker than this are not boosted.
pub const MIN_BOOSTED_STRENGTH: f32 = 0.05;
/// How often, in game seconds, a creature looks for threats.
pub const SCAN_INTERVAL: f64 = 0.1;

/// The sanitized config values the panic logic uses.
#[derive(Debug, Clone, Copy)]
pub struct Params {
    pub evade: bool,
    pub reaction_probability: f32,
    pub burst_seconds: f32,
    pub min_interval_seconds: f32,
    pub evasion_seconds: f32,
    pub evasion_strength: f32,
    pub barrel_roll_probability: f32,
    pub barrel_roll_seconds: f32,
    pub panic_evasion_probability: f32,
    pub panic_evasion_seconds: f32,
}

/// A missile that is locked on to a ship and inside the trigger range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Threat {
    /// Centre-to-centre distance as the game's own distance routine gives it (used for the trace only).
    pub distance: f32,
    pub position: (f32, f32),
    /// Address of the missile body (used for the trace only; the hand-over for later velocity logic).
    pub body: usize,
    /// Not locked on this ship, but on a collision course: evade without dumping flares.
    pub stray: bool,
}

pub struct Ship {
    pub next_scan: f64,
    /// Flares are dropped until this game time.
    pub burst_until: f64,
    /// The ship evades until this game time.
    pub evade_until: f64,
    /// No new panic starts before this game time.
    pub quiet_until: f64,
    /// This panic is a barrel roll: the ship spins instead of evading.
    pub roll: bool,
    /// Game time the barrel roll started.
    pub roll_start: f64,
    /// This panic is a panic evasion: boost, full steering takeover, one fixed direction.
    pub panic: bool,
    /// The direction a panic evasion has committed to (unit vector).
    pub flight: Option<(f32, f32)>,
    /// `1.0` or `-1.0`: which side of the missile's line of approach the ship runs to (and which
    /// way a barrel roll turns).
    pub side: f32,
    /// Where the nearest threatening missile was at the last scan.
    pub threat: Option<(f32, f32)>,
    // Trace-only bookkeeping (hf_ai addition; never read by the behaviour).
    /// A panic start was traced and its end has not been traced yet.
    pub trace_active: bool,
    pub trace_last_steer: f64,
    pub trace_last_engine: f64,
}

impl Default for Ship {
    fn default() -> Self {
        Self {
            next_scan: 0.0,
            burst_until: 0.0,
            evade_until: 0.0,
            quiet_until: 0.0,
            roll: false,
            roll_start: 0.0,
            panic: false,
            flight: None,
            side: 1.0,
            threat: None,
            trace_active: false,
            trace_last_steer: f64::NEG_INFINITY,
            trace_last_engine: f64::NEG_INFINITY,
        }
    }
}

impl Ship {
    /// Whether the ship is in a panic (flares dropping or evading).
    pub fn panicking(&self, now: f64) -> bool {
        now < self.burst_until || now < self.evade_until
    }

    /// Whether the launchers should be fired this frame.
    pub fn firing(&self, now: f64) -> bool {
        now < self.burst_until
    }

    /// Whether the per-ship state can be dropped (nothing running and no quiet period left).
    pub fn idle(&self, now: f64) -> bool {
        !(self.burst_until > now || self.evade_until > now || self.quiet_until > now)
    }
}

/// What a scan of one ship did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScanEvent {
    /// Not time to scan yet, or the ship is in its quiet period.
    NotDue,
    /// Scanned, nothing threatening (or a panic running with no missile left).
    Clear,
    /// A panic is running and a missile is still coming.
    Seen(Threat),
    /// A missile is in range but the reaction roll failed.
    Ignored(Threat),
    /// A panic started.
    Started(Threat),
}

fn random_side(rand: &mut dyn FnMut() -> f32) -> f32 {
    if rand() < 0.5 {
        -1.0
    } else {
        1.0
    }
}

/// The scan-and-timers block of the original `on_creature_update`. `find_threat` is the game-memory scan
/// (`threatened`) and only runs when a scan is due; `rand` is the `[0, 1)` generator. The order of the dice rolls
/// is the original's.
pub fn scan(
    ship: &mut Ship,
    now: f64,
    p: &Params,
    find_threat: impl FnOnce() -> Option<Threat>,
    rand: &mut dyn FnMut() -> f32,
) -> ScanEvent {
    let panicking = ship.panicking(now);

    if now >= ship.next_scan && (panicking || now >= ship.quiet_until) {
        ship.next_scan = now + SCAN_INTERVAL;
        let threat = find_threat();
        if panicking {
            // A missile that is still coming keeps the evasion going.
            if let Some(found) = threat {
                ship.threat = Some(found.position);
                // A stray starts evasion only. If a locked missile appears during that evasion, add one flare burst;
                // a burst that already ran has a non-zero expiry and is not restarted on every scan.
                if !found.stray && ship.burst_until == 0.0 {
                    ship.burst_until = now + f64::from(p.burst_seconds);
                }
                if p.evade && !ship.roll {
                    let extension = if ship.panic {
                        PANIC_EVASION_EXTENSION
                    } else {
                        EVASION_EXTENSION
                    };
                    ship.evade_until = ship.evade_until.max(now + extension);
                    ship.quiet_until =
                        ship.burst_until.max(ship.evade_until) + f64::from(p.min_interval_seconds);
                }
                return ScanEvent::Seen(found);
            }
        } else if let Some(found) = threat {
            // EXTENSION POINT (threat decision): `found` carries the missile body address, its position and its
            // distance. Time-to-impact / missile-velocity logic ("is this missile worth a panic?", the overshoot
            // dodge) would go here, between the threat and the reaction roll. Deliberately not done in v0.2.0:
            // the port is behaviour-identical to qol `ai_flares`.
            if rand() < p.reaction_probability {
                ship.threat = Some(found.position);
                ship.burst_until = if found.stray { 0.0 } else { now + f64::from(p.burst_seconds) };
                ship.roll = p.evade && rand() < p.barrel_roll_probability;
                ship.roll_start = now;
                ship.panic = p.evade && !ship.roll && rand() < p.panic_evasion_probability;
                ship.evade_until = if !p.evade {
                    0.0
                } else if ship.roll {
                    now + f64::from(p.barrel_roll_seconds)
                } else if ship.panic {
                    now + f64::from(p.panic_evasion_seconds)
                } else {
                    now + f64::from(p.evasion_seconds)
                };
                ship.flight = None;
                ship.side = random_side(rand);
                ship.quiet_until =
                    ship.burst_until.max(ship.evade_until) + f64::from(p.min_interval_seconds);
                return ScanEvent::Started(found);
            }
            return ScanEvent::Ignored(found);
        }
        return ScanEvent::Clear;
    }
    ScanEvent::NotDue
}

pub fn normalized(x: f32, y: f32) -> Option<(f32, f32, f32)> {
    let length = (x * x + y * y).sqrt();
    (length.is_finite() && length > 1.0e-4).then(|| (x / length, y / length, length))
}

/// Smallest missile speed (u/s) for which a flight path is extrapolated.
pub const STRAY_MIN_SPEED: f32 = 60.0;

/// Does a missile flying in a straight line at `velocity` from `missile` pass within `radius` of `ship` within
/// `eta_max` seconds? Returns `(time of closest approach, miss distance)`. For a missile that is not locked on to
/// the ship (a stray: lost lock, or aimed at someone else) this decides whether the ship is in its way.
pub fn stray_cpa(
    ship: (f32, f32),
    missile: (f32, f32),
    velocity: (f32, f32),
    radius: f32,
    eta_max: f32,
) -> Option<(f32, f32)> {
    let speed2 = velocity.0 * velocity.0 + velocity.1 * velocity.1;
    if !(speed2.is_finite() && speed2 >= STRAY_MIN_SPEED * STRAY_MIN_SPEED) {
        return None;
    }
    let rel = (ship.0 - missile.0, ship.1 - missile.1);
    let t = (rel.0 * velocity.0 + rel.1 * velocity.1) / speed2;
    if !(t.is_finite() && t >= 0.0 && t <= eta_max) {
        return None;
    }
    let miss = (rel.0 - velocity.0 * t).hypot(rel.1 - velocity.1 * t);
    (miss.is_finite() && miss <= radius).then_some((t, miss))
}

/// Above this y (the ground is at about +250; y grows downward) a ship in a panic may not steer downward: flares drop
/// BELOW the ship, so the way out is up or sideways, never into the ground or the flares.
pub const NO_DESCENT_BELOW_Y: f32 = -250.0;
/// The two sidestep candidates must differ by at least this in y for the higher one to be preferred over `side`.
const UP_PREFERENCE_MARGIN: f32 = 0.15;

/// Pulls the ship's waypoint towards an evasive one. The evasive direction is a sidestep
/// perpendicular to the line to the missile (to `side`) with half as much retreat mixed in; the
/// result keeps the original waypoint's distance from the ship. Returns `None` when the numbers
/// make no sense, in which case the waypoint is left alone.
pub fn evade_waypoint(
    waypoint: (f32, f32),
    ship: (f32, f32),
    missile: (f32, f32),
    side: f32,
    strength: f32,
) -> Option<(f32, f32)> {
    let (course_x, course_y, course_length) = normalized(waypoint.0 - ship.0, waypoint.1 - ship.1)
        .unwrap_or((0.0, 0.0, EVASION_FALLBACK_LENGTH));
    let (to_missile_x, to_missile_y, _) = normalized(missile.0 - ship.0, missile.1 - ship.1)?;

    // Perpendicular to the line to the missile, plus a retreat from it. Of the two sides the HIGHER one wins when they
    // differ clearly (flares fall below the ship, ground is below it); `side` decides when they are about equal.
    let candidate = |s: f32| normalized(-to_missile_y * s - to_missile_x * EVASION_RETREAT, to_missile_x * s - to_missile_y * EVASION_RETREAT);
    let (preferred, other) = (candidate(side)?, candidate(-side)?);
    let chosen = if other.1 < preferred.1 - UP_PREFERENCE_MARGIN { other } else { preferred };
    let (mut evade_x, mut evade_y) = (chosen.0, chosen.1);
    if ship.1 > NO_DESCENT_BELOW_Y && evade_y > 0.0 {
        // low: no downward component at all, keep the sideways part
        evade_y = 0.0;
        evade_x = if evade_x.abs() > 1.0e-3 { evade_x.signum() } else { side };
    }

    let (mut x, mut y, _) = normalized(
        course_x + (evade_x - course_x) * strength,
        course_y + (evade_y - course_y) * strength,
    )?;
    if strength > 0.0 && ship.1 > NO_DESCENT_BELOW_Y && y > 0.0 {
        // the blend with a downward stock course can still point down: level out
        y = 0.0;
        x = if x.abs() > 1.0e-3 { x.signum() } else { evade_x.signum() };
    }
    let course_length = if course_length > 1.0 {
        course_length
    } else {
        EVASION_FALLBACK_LENGTH
    };

    Some((ship.0 + x * course_length, ship.1 + y * course_length))
}

/// Result of [`steer`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SteerOutcome {
    /// The replacement waypoint, `None` = leave it.
    pub waypoint: Option<(f32, f32)>,
    /// A panic evasion committed to a direction on this call (first steering call of the panic).
    pub new_flight: bool,
}

/// The waypoint block of the original `on_steering` (everything after the per-ship lookup).
/// `waypoint` and `ship_xy` are what the AI tick wants / where the ship is.
pub fn steer(
    ship: &mut Ship,
    now: f64,
    p: &Params,
    waypoint: (f32, f32),
    ship_xy: (f32, f32),
) -> SteerOutcome {
    let none = SteerOutcome::default();
    let (waypoint_x, waypoint_y) = waypoint;
    let (ship_x, ship_y) = ship_xy;
    if now >= ship.evade_until {
        return none;
    }
    let Some(missile) = ship.threat else {
        return none;
    };

    if ship.roll {
        // The engines are commanded directly (see `on_engine_commands`).
        none
    } else if ship.panic {
        // Pick one escape direction when the panic starts and floor it that way.
        let mut new_flight = false;
        if ship.flight.is_none() {
            ship.flight = evade_waypoint(
                (waypoint_x, waypoint_y),
                (ship_x, ship_y),
                missile,
                ship.side,
                1.0,
            )
            .and_then(|(x, y)| normalized(x - ship_x, y - ship_y))
            .map(|(x, y, _)| (x, y));
            new_flight = ship.flight.is_some();
        }
        let Some((flight_x, flight_y)) = ship.flight else {
            return none;
        };
        let length = normalized(waypoint_x - ship_x, waypoint_y - ship_y)
            .map_or(EVASION_FALLBACK_LENGTH, |(_, _, length)| length.max(1.0));
        SteerOutcome {
            waypoint: Some((ship_x + flight_x * length, ship_y + flight_y * length)),
            new_flight,
        }
    } else {
        let Some(new_waypoint) = evade_waypoint(
            (waypoint_x, waypoint_y),
            (ship_x, ship_y),
            missile,
            ship.side,
            p.evasion_strength,
        ) else {
            return none;
        };
        SteerOutcome { waypoint: Some(new_waypoint), new_flight: false }
    }
}

/// The ship lookup of the original `on_engine_commands`: `Some((roll, side))` while the ship is in a panic
/// evasion or a barrel roll.
pub fn engine_mode(ship: &Ship, now: f64) -> Option<(bool, f32)> {
    if (ship.panic || ship.roll) && now < ship.evade_until {
        Some((ship.roll, ship.side))
    } else {
        None
    }
}

/// New strength of one engine command in a panic evasion, `None` = leave it.
pub fn boosted_strength(current: f32, boost: f32) -> Option<f32> {
    // A command with (almost) no strength has no meaningful direction: leave it off.
    (current.is_finite() && current > MIN_BOOSTED_STRENGTH).then(|| current.max(1.0) * boost)
}

/// The two command angles of a barrel roll for a ship with `heading` (radians): `(first, second)` go to the
/// `0x66`/`0x64` and the `0x67`/`0x65` commands. `None` when the heading is not a number.
pub fn spin_angles(heading: f32, side: f32) -> Option<(f32, f32)> {
    if !heading.is_finite() {
        return None;
    }
    let opposite = if heading > 0.0 {
        heading - std::f32::consts::PI
    } else {
        heading + std::f32::consts::PI
    };
    Some(if side > 0.0 {
        (heading, opposite)
    } else {
        (opposite, heading)
    })
}
