use serde::{Deserialize, Serialize};

/// Largest accepted trigger range, in game units.
pub const MAX_TRIGGER_RANGE: f32 = 5000.0;
/// Largest accepted panic engine boost.
pub const MAX_PANIC_BOOST: f32 = 4.0;
/// Largest accepted burst, evasion or interval time, in seconds.
pub const MAX_SECONDS: f32 = 60.0;

/// Settings for the `ai_flares` block of `hf_ai.json` (ported from the qol fork's `ai_flares` section).
///
/// The game's AI ships carry flare launchers but never use them. With this section on, an AI ship
/// panics when a missile that has locked on to it comes within `trigger_range`: it dumps flares and
/// throws itself sideways until the missile is gone.
///
/// Only has an effect on Steam 1.163. Defaults match config/hf_ai.json. Keep any separate QoL ai_flares feature off:
/// both patch the same sites. The block also controls missile evasion, barrel rolls and stray-missile reactions, not
/// only flare releases. The preventive flare curtain was retired in v0.10.18; old `curtain_*` and `launch_flares_*`
/// keys are ignored by the config parser.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiFlaresConfig {
    /// Turns AI missile defence on. Enabled by player default. Enable it only when qol's `ai_flares.enabled` is
    /// `false`: both patch the same three sites and whichever loads second finds other bytes and skips.
    pub enabled: bool,
    /// How close, in game units, an incoming missile has to be before the ship reacts.
    ///
    /// Limited to `0` through `5000`.
    pub trigger_range: f32,
    /// Chance that the ship reacts to a given missile. Rolled once when that missile first
    /// enters the trigger range; a failed roll makes the ship ignore that missile until the
    /// immediate threat picture clears. Other and later missiles roll separately. `1.0` always
    /// reacts, `0.1` lets most missiles through.
    ///
    /// Limited to `0.0` through `1.0`.
    pub reaction_probability: f32,
    /// How long, in seconds, the launchers keep dropping flares once the panic starts. Each
    /// launcher drops one flare per its own firing interval (0.15 s for the vanilla launcher), so
    /// 1.5 s is about ten flares per launcher.
    ///
    /// Limited to `0.05` through `60`.
    pub burst_seconds: f32,
    /// Quiet time, in seconds, after a panic ends before the same ship may panic again.
    ///
    /// Limited to `0` through `60`.
    pub min_interval_seconds: f32,
    /// Makes the ship also manoeuvre while it panics. With `false` it only drops flares.
    pub evade: bool,
    /// Lets AI-run ships of the player's own side (allies brought in by a mod, wingmen) use flares
    /// and evasion too. The game's own check is "negative side = enemy", so friendly AI ships never
    /// did. The ship you fly yourself is never affected. Enabled by default.
    pub allies: bool,
    /// How long, in seconds, the evasive manoeuvre lasts. It is extended by one second each time
    /// a locked-on missile is still inside the range.
    ///
    /// Limited to `0.05` through `60`.
    pub evasion_seconds: f32,
    /// How much of the ship's course the evasion takes over: `0.0` none, `1.0` all of it.
    ///
    /// Limited to `0.0` through `1.0`.
    pub evasion_strength: f32,
    /// Chance, rolled once when a panic starts, that the captain panics so hard the ship does a
    /// panic spin (what the player's Q or E with Shift does: the engines on one side push up and
    /// the others down, boosted by `panic_boost`; flares still drop) instead of evading.
    ///
    /// Limited to `0.0` through `1.0`.
    pub barrel_roll_probability: f32,
    /// How long, in seconds, the panic spin lasts. It is not extended.
    ///
    /// Limited to `0.05` through `60`.
    pub barrel_roll_seconds: f32,
    /// Chance, rolled once when a panic starts, that the evasion is a *panic evasion*: the ship
    /// ignores its normal steering completely, picks one escape direction and floors it that way
    /// and boosts its engines like a player holding Shift. The other panics use
    /// the plain evasion above.
    ///
    /// Limited to `0.0` through `1.0`.
    pub panic_evasion_probability: f32,
    /// Engine strength during a panic evasion: `1.0` is normal thrust, `2.0` is what the player's
    /// boost key gives.
    ///
    /// Limited to `1.0` through `4.0`.
    pub panic_boost: f32,
    /// How long, in seconds, a panic evasion lasts. A missile that is still coming extends it by
    /// 2.5 s at a time.
    ///
    /// Limited to `0.05` through `60`.
    pub panic_evasion_seconds: f32,
    /// Writes `Modloader/logs/hf_ai_panic_<unix>.log`: one line per panic event (start, extension, end, ignored
    /// reaction roll, first panic-evasion direction, spin/boost) with the game time and the ship pointer.
    /// Disabled by default.
    #[serde(skip)]
    pub log_panic: bool,
    /// Stray missiles: a missile that is NOT locked on to the ship (it lost its lock or is aimed at someone else) but
    /// flies a path that passes within `stray_missile_radius` (+ half the ship's size) of the ship within
    /// `stray_missile_eta_s` seconds, closer than `stray_missile_range`, counts as a threat for the panic.
    pub stray_missiles: bool,
    pub stray_missile_radius: f32,
    pub stray_missile_eta_s: f32,
    pub stray_missile_range: f32,
}

impl Default for AiFlaresConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            trigger_range: 400.0,
            reaction_probability: 0.6,
            burst_seconds: 1.5,
            min_interval_seconds: 1.5,
            evade: true,
            allies: true,
            evasion_seconds: 3.0,
            evasion_strength: 0.5,
            barrel_roll_probability: 0.001,
            barrel_roll_seconds: 2.0,
            panic_evasion_probability: 0.1,
            panic_boost: 2.0,
            panic_evasion_seconds: 5.0,
            log_panic: false,
            stray_missiles: true,
            stray_missile_radius: 150.0,
            stray_missile_eta_s: 3.0,
            stray_missile_range: 1200.0,
        }
    }
}

impl AiFlaresConfig {
    /// Returns a copy with every value limited to its accepted range. Numbers that are not
    /// finite fall back to the default.
    pub fn sanitized(&self) -> Self {
        let default = Self::default();
        let pick = |value: f32, fallback: f32, min: f32, max: f32| {
            if value.is_finite() {
                value.clamp(min, max)
            } else {
                fallback
            }
        };

        Self {
            enabled: self.enabled,
            trigger_range: pick(
                self.trigger_range,
                default.trigger_range,
                0.0,
                MAX_TRIGGER_RANGE,
            ),
            reaction_probability: pick(
                self.reaction_probability,
                default.reaction_probability,
                0.0,
                1.0,
            ),
            burst_seconds: pick(self.burst_seconds, default.burst_seconds, 0.05, MAX_SECONDS),
            min_interval_seconds: pick(
                self.min_interval_seconds,
                default.min_interval_seconds,
                0.0,
                MAX_SECONDS,
            ),
            evade: self.evade,
            allies: self.allies,
            log_panic: self.log_panic,
            evasion_seconds: pick(
                self.evasion_seconds,
                default.evasion_seconds,
                0.05,
                MAX_SECONDS,
            ),
            evasion_strength: pick(self.evasion_strength, default.evasion_strength, 0.0, 1.0),
            barrel_roll_seconds: pick(
                self.barrel_roll_seconds,
                default.barrel_roll_seconds,
                0.05,
                MAX_SECONDS,
            ),
            barrel_roll_probability: pick(
                self.barrel_roll_probability,
                default.barrel_roll_probability,
                0.0,
                1.0,
            ),
            panic_evasion_probability: pick(
                self.panic_evasion_probability,
                default.panic_evasion_probability,
                0.0,
                1.0,
            ),
            panic_boost: pick(self.panic_boost, default.panic_boost, 1.0, MAX_PANIC_BOOST),
            panic_evasion_seconds: pick(
                self.panic_evasion_seconds,
                default.panic_evasion_seconds,
                0.05,
                MAX_SECONDS,
            ),
            stray_missiles: self.stray_missiles,
            stray_missile_radius: pick(self.stray_missile_radius, default.stray_missile_radius, 10.0, 1000.0),
            stray_missile_eta_s: pick(self.stray_missile_eta_s, default.stray_missile_eta_s, 0.2, 20.0),
            stray_missile_range: pick(self.stray_missile_range, default.stray_missile_range, 100.0, MAX_TRIGGER_RANGE * 2.0),
        }
    }
}
