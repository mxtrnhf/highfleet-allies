//! The pure part of the ported qol `ai_flares`: direction math and the per-ship panic timers. No game memory, no
//! dependencies, so it can be tested on the host (`rustc --edition 2021 --test src/panic_math.rs`, see README).
//! The function bodies started as the original `hf_qol_fork/src/ai_flares.rs` code, moved here unchanged except
//! where noted; the hook glue that reads game memory stays in `ai_flares.rs`. The reaction roll is now per missile
//! body instead of per scan: a failed roll is remembered in [`PanicState::ignored_threats`] until the threat picture
//! clears.

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
/// Upper bound on the missile bodies a ship remembers as ignored after a failed reaction roll.
const MAX_IGNORED_THREATS: usize = 16;

/// The sanitized config values the panic logic uses.
#[derive(Debug, Clone, Copy)]
pub struct PanicParams {
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

/// The result of looking for threats around one ship.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThreatScan {
    /// The nearest threat that has not already failed its reaction roll.
    pub threat: Option<Threat>,
    /// Whether any valid threat was in range, including missiles this ship already ignored.
    pub any_in_range: bool,
}

pub struct PanicState {
    pub next_scan: f64,
    /// Flares are dropped until this game time.
    pub flare_burst_until: f64,
    /// The ship evades until this game time.
    pub maneuver_until: f64,
    /// No new panic starts before this game time.
    pub cooldown_until: f64,
    /// This panic is a barrel roll: the ship spins instead of evading.
    pub barrel_roll: bool,
    /// This panic is a panic evasion: boost, full steering takeover, one fixed direction.
    pub panic_evasion: bool,
    /// The direction a panic evasion has committed to (unit vector).
    pub flight: Option<(f32, f32)>,
    /// `1.0` or `-1.0`: which side of the missile's line of approach the ship runs to (and which
    /// way a barrel roll turns).
    pub dodge_side: f32,
    /// Where the nearest threatening missile was at the last scan.
    pub missile_position: Option<(f32, f32)>,
    /// Missile bodies whose reaction roll failed. They stay ignored until no threat is left in range.
    pub ignored_threats: Vec<usize>,
    // Trace-only bookkeeping (hf_ai addition; never read by the behaviour).
    /// A panic start was traced and its end has not been traced yet.
    pub trace_active: bool,
    pub trace_last_steer: f64,
    pub trace_last_engine: f64,
}

impl Default for PanicState {
    fn default() -> Self {
        Self {
            next_scan: 0.0,
            flare_burst_until: 0.0,
            maneuver_until: 0.0,
            cooldown_until: 0.0,
            barrel_roll: false,
            panic_evasion: false,
            flight: None,
            dodge_side: 1.0,
            missile_position: None,
            ignored_threats: Vec::new(),
            trace_active: false,
            trace_last_steer: f64::NEG_INFINITY,
            trace_last_engine: f64::NEG_INFINITY,
        }
    }
}

impl PanicState {
    /// Whether the ship is in a panic (flares dropping or evading).
    pub fn is_panicking(&self, now: f64) -> bool {
        now < self.flare_burst_until || now < self.maneuver_until
    }

    /// Whether the launchers should be fired this frame.
    pub fn releasing_flares(&self, now: f64) -> bool {
        now < self.flare_burst_until
    }

    /// Whether the per-ship state can be dropped (nothing running and no quiet period left).
    pub fn is_idle(&self, now: f64) -> bool {
        !(self.flare_burst_until > now || self.maneuver_until > now || self.cooldown_until > now)
    }
}

/// What a scan of one ship did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ThreatEvent {
    /// Not time to scan yet, or the ship is in its quiet period.
    NotDue,
    /// Scanned, nothing threatening (or a panic running with no missile left).
    NoThreat,
    /// A panic is running and a missile is still coming.
    ThreatContinues(Threat),
    /// A new missile entered range and its reaction roll failed; it is now ignored.
    ReactionRollFailed(Threat),
    /// A panic started.
    PanicStarted(Threat),
}

fn random_side(rand: &mut dyn FnMut() -> f32) -> f32 {
    if rand() < 0.5 {
        -1.0
    } else {
        1.0
    }
}

/// The scan-and-timers block of the original `on_creature_update`. `find_threat` is the game-memory scan
/// (`threatened`) and only runs when a scan is due; it receives the missile bodies this ship already ignored and
/// returns the nearest remaining threat. `rand` is the `[0, 1)` generator. The order of the dice rolls is the
/// original's, except that the reaction roll is made once per missile body rather than once per scan.
pub fn scan(
    ship: &mut PanicState,
    now: f64,
    p: &PanicParams,
    find_threat: impl FnOnce(&[usize]) -> ThreatScan,
    rand: &mut dyn FnMut() -> f32,
) -> ThreatEvent {
    let panicking = ship.is_panicking(now);

    if now >= ship.next_scan && (panicking || now >= ship.cooldown_until) {
        ship.next_scan = now + SCAN_INTERVAL;
        let scan = find_threat(&ship.ignored_threats);
        if panicking {
            // A missile that is still coming keeps the evasion going.
            if let Some(found) = scan.threat {
                ship.missile_position = Some(found.position);
                // A stray starts evasion only. If a locked missile appears during that evasion, add one flare burst;
                // a burst that already ran has a non-zero expiry and is not restarted on every scan.
                if !found.stray && ship.flare_burst_until == 0.0 {
                    ship.flare_burst_until = now + f64::from(p.burst_seconds);
                }
                if p.evade && !ship.barrel_roll {
                    let extension = if ship.panic_evasion {
                        PANIC_EVASION_EXTENSION
                    } else {
                        EVASION_EXTENSION
                    };
                    ship.maneuver_until = ship.maneuver_until.max(now + extension);
                    ship.cooldown_until =
                        ship.flare_burst_until.max(ship.maneuver_until) + f64::from(p.min_interval_seconds);
                }
                return ThreatEvent::ThreatContinues(found);
            }
        } else if let Some(found) = scan.threat {
            // EXTENSION POINT (threat decision): `found` carries the missile body address, its position and its
            // distance. Time-to-impact / missile-velocity logic ("is this missile worth a panic?", the overshoot
            // dodge) would go here, between the threat and the reaction roll.
            if rand() < p.reaction_probability {
                ship.missile_position = Some(found.position);
                ship.flare_burst_until = if found.stray { 0.0 } else { now + f64::from(p.burst_seconds) };
                ship.barrel_roll = p.evade && rand() < p.barrel_roll_probability;
                ship.panic_evasion = p.evade && !ship.barrel_roll && rand() < p.panic_evasion_probability;
                ship.maneuver_until = if !p.evade {
                    0.0
                } else if ship.barrel_roll {
                    now + f64::from(p.barrel_roll_seconds)
                } else if ship.panic_evasion {
                    now + f64::from(p.panic_evasion_seconds)
                } else {
                    now + f64::from(p.evasion_seconds)
                };
                ship.flight = None;
                ship.dodge_side = random_side(rand);
                ship.cooldown_until =
                    ship.flare_burst_until.max(ship.maneuver_until) + f64::from(p.min_interval_seconds);
                return ThreatEvent::PanicStarted(found);
            }
            if ship.ignored_threats.len() >= MAX_IGNORED_THREATS {
                ship.ignored_threats.remove(0);
            }
            if !ship.ignored_threats.contains(&found.body) {
                ship.ignored_threats.push(found.body);
            }
            return ThreatEvent::ReactionRollFailed(found);
        } else if !scan.any_in_range {
            // The missile picture is clear again, so a later approach starts with fresh rolls.
            ship.ignored_threats.clear();
        }
        return ThreatEvent::NoThreat;
    }
    ThreatEvent::NotDue
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

/// Below this y (the ground is at about +250; y grows downward) a ship in a panic may not steer downward: flares drop
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

/// Result of [`evasive_waypoint`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EvasionOutcome {
    /// The replacement waypoint, `None` = leave it.
    pub waypoint: Option<(f32, f32)>,
    /// A panic evasion committed to a direction on this call (first steering call of the panic).
    pub new_flight: bool,
}

/// The waypoint block of the original `on_steering` (everything after the per-ship lookup).
/// `waypoint` and `ship_xy` are what the AI tick wants / where the ship is.
pub fn evasive_waypoint(
    ship: &mut PanicState,
    now: f64,
    p: &PanicParams,
    waypoint: (f32, f32),
    ship_xy: (f32, f32),
) -> EvasionOutcome {
    let none = EvasionOutcome::default();
    let (waypoint_x, waypoint_y) = waypoint;
    let (ship_x, ship_y) = ship_xy;
    if now >= ship.maneuver_until {
        return none;
    }
    let Some(missile) = ship.missile_position else {
        return none;
    };

    if ship.barrel_roll {
        // The engines are commanded directly (see `on_engine_commands`).
        none
    } else if ship.panic_evasion {
        // Pick one escape direction when the panic starts and floor it that way.
        let mut new_flight = false;
        if ship.flight.is_none() {
            ship.flight = evade_waypoint(
                (waypoint_x, waypoint_y),
                (ship_x, ship_y),
                missile,
                ship.dodge_side,
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
        EvasionOutcome {
            waypoint: Some((ship_x + flight_x * length, ship_y + flight_y * length)),
            new_flight,
        }
    } else {
        let Some(new_waypoint) = evade_waypoint(
            (waypoint_x, waypoint_y),
            (ship_x, ship_y),
            missile,
            ship.dodge_side,
            p.evasion_strength,
        ) else {
            return none;
        };
        EvasionOutcome { waypoint: Some(new_waypoint), new_flight: false }
    }
}

/// The ship lookup of the original `on_engine_commands`: `Some((barrel_roll, dodge_side))` while the ship is in a panic
/// evasion or a barrel roll.
pub fn engine_mode(ship: &PanicState, now: f64) -> Option<(bool, f32)> {
    if (ship.panic_evasion || ship.barrel_roll) && now < ship.maneuver_until {
        Some((ship.barrel_roll, ship.dodge_side))
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
