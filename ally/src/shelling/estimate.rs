//! Campaign balance model for standard HE; all distances and times are on the strategic map.

const LANDED_RANGE_KM: f32 = 270.0;
const CRUISE_RANGE_KM: f32 = 450.0;
const CRUISE_ALTITUDE_M: f32 = 9_000.0;
/// Chance that one shell damages a useful target inside the city. The remaining
/// rounds fall inside the built-up area but miss anything worth counting.
const CLOSE_ROUND_HIT_CHANCE: f64 = 0.02;
const EDGE_ROUND_HIT_CHANCE: f64 = 0.005;
const SECONDS_PER_MINUTE: f64 = 60.0;
// Strategic map pacing, not tactical projectile speed. Equal distances mean equal ETAs.
const MAP_SHELL_SPEED_KM_PER_SECOND: f64 = 1.0;
const MIN_FLIGHT_SECONDS: f64 = 5.0;
/// Bound the firing plan even with badly modified weapon stats.
pub(super) const MAX_SIMULATED_ROUNDS: u32 = 250_000;

pub(super) fn flight_seconds(distance_km: f32) -> f64 {
    (f64::from(distance_km) / MAP_SHELL_SPEED_KM_PER_SECOND).max(MIN_FLIGHT_SECONDS)
}

#[derive(Clone, Copy, Default)]
pub(super) enum Duration {
    #[default]
    Short,
    Medium,
    Long,
}

impl Duration {
    pub(super) fn next(self) -> Self {
        match self {
            Self::Short => Self::Medium,
            Self::Medium => Self::Long,
            Self::Long => Self::Short,
        }
    }

    pub(super) fn minutes(self) -> u32 {
        match self {
            Self::Short => 10,
            Self::Medium => 30,
            Self::Long => 60,
        }
    }
}

/// One mount, starting loaded, using its base rate and loading one round at a time.
/// The firing window includes its start but not its end. The magazine can absorb a
/// burst, but subsequent shots are limited by both the fire timer and the reload supply.
pub(super) fn mount_rounds(
    duration: Duration,
    capacity: i32,
    reload_seconds: f32,
    rounds_per_minute: f32,
) -> Option<u32> {
    let capacity = u32::try_from(capacity).ok()?;
    if capacity == 0
        || !reload_seconds.is_finite()
        || reload_seconds < 0.0
        || !rounds_per_minute.is_finite()
        || rounds_per_minute <= 0.0
    {
        return None;
    }
    let minutes = f64::from(duration.minutes());
    let fire_limit = (minutes * f64::from(rounds_per_minute)).ceil() as u32;
    // A zero load time disables the native reload path, not the initial magazine.
    let reloaded = if reload_seconds == 0.0 {
        0
    } else {
        let ticks = minutes * SECONDS_PER_MINUTE * f64::from(capacity) / f64::from(reload_seconds);
        (ticks.ceil() - 1.0) as u32
    };
    Some(fire_limit.min(capacity.saturating_add(reloaded)))
}

pub(super) enum Estimate {
    OutOfRange {
        distance_km: f32,
        range_km: f32,
    },
    InRange {
        rounds: u32,
        first_impact_minutes: f32,
        expected_hits: f64,
        any_hit_percent: f64,
    },
}

/// Produces a firing preview, not a damage roll. HE is unlimited, so the round count is
/// informational only. Altitude extends reach; it does not change the campaign travel speed.
pub(super) fn estimate(distance_km: f32, altitude_m: f32, rounds: u32) -> Option<Estimate> {
    if !distance_km.is_finite() || distance_km < 0.0 || !altitude_m.is_finite() || rounds == 0 {
        return None;
    }
    let altitude_fraction = (altitude_m / CRUISE_ALTITUDE_M).clamp(0.0, 1.0);
    let range_km = LANDED_RANGE_KM + (CRUISE_RANGE_KM - LANDED_RANGE_KM) * altitude_fraction;
    if distance_km > range_km {
        return Some(Estimate::OutOfRange {
            distance_km,
            range_km,
        });
    }
    let round_chance = round_hit_chance(distance_km, altitude_m);
    // Independent rounds: the chance of at least one hit rises with volume while
    // the expected number of hits is the quantity combat resolution consumes.
    let any_hit = 1.0 - (1.0 - round_chance).powi(i32::try_from(rounds).unwrap_or(i32::MAX));
    Some(Estimate::InRange {
        rounds,
        first_impact_minutes: (flight_seconds(distance_km) / SECONDS_PER_MINUTE) as f32,
        expected_hits: f64::from(rounds) * round_chance,
        any_hit_percent: 100.0 * any_hit,
    })
}

/// Independent probability for one actual round, shared by the preview and firing simulation.
pub(super) fn round_hit_chance(distance_km: f32, altitude_m: f32) -> f64 {
    let altitude_fraction = (altitude_m / CRUISE_ALTITUDE_M).clamp(0.0, 1.0);
    let range_km = LANDED_RANGE_KM + (CRUISE_RANGE_KM - LANDED_RANGE_KM) * altitude_fraction;
    let range_fraction = (distance_km / range_km).clamp(0.0, 1.0);
    CLOSE_ROUND_HIT_CHANCE
        - (CLOSE_ROUND_HIT_CHANCE - EDGE_ROUND_HIT_CHANCE) * f64::from(range_fraction.powi(2))
}

/// Resolve effective rounds once. Both the strike and the map display use these results.
pub(super) fn effective_rounds(seed: u64, rounds: u32, chance: f64) -> Vec<u32> {
    (0..rounds)
        .filter(|&index| (round_roll(seed, index) >> 40) as f64 / f64::from(1u32 << 24) < chance)
        .collect()
}

/// Deterministic per-round sample, independent of presentation and its random draws.
pub(super) fn round_roll(seed: u64, index: u32) -> u64 {
    let mut value = seed ^ f64::from(index).to_bits() ^ u64::from(index).wrapping_mul(0x9e37_79b9);
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}
