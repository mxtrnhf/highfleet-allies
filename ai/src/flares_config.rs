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
/// Only has an effect on Steam 1.163. Defaults match config/hf_ai.json. Keep any separate QoL ai_flares feature off;
/// exactly one implementation may own the flare hooks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiFlaresConfig {
    /// Turns AI flares on. Enabled by player default. Enable it only when qol's `ai_flares.enabled` is `false`: both
    /// patch the same three sites and whichever loads second finds other bytes and skips.
    pub enabled: bool,
    /// How close, in game units, an incoming missile has to be before the ship reacts.
    ///
    /// Limited to `0` through `5000`.
    pub trigger_range: f32,
    /// Chance, checked about ten times a second while a missile is inside the range, that the
    /// ship starts a burst. `1.0` reacts at once, `0.1` makes the ship slow and unreliable.
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
    pub log_panic: bool,
    /// "Flare curtain": chance, rolled once per enemy ship each time the player's side launches a missile, that the
    /// ship starts dropping flares at random intervals BEFORE any lock-on (like a helicopter flooding the area with
    /// fake heat sources). `0.0` = off. Enemy ships only.
    ///
    /// Limited to `0.0` through `1.0`.
    pub launch_flares_probability: f32,
    /// Shortest and longest wait between two drops of a curtain, in seconds (every drop fires all launchers).
    pub launch_flares_interval_min: f32,
    pub launch_flares_interval_max: f32,
    /// How long one curtain lasts; another launch while it runs extends it.
    pub launch_flares_seconds: f32,
    /// Most drops of one curtain, so the racks are not empty when the real panic needs them.
    pub launch_flares_max_drops: u32,
    /// Quiet time after a curtain before the next one may start.
    pub launch_flares_cooldown: f32,
    /// While a curtain runs the ship moves away from the launched missile: `0.0` none, `1.0` all of its course.
    pub launch_flares_evade: f32,
    /// Only missiles closer than this (game units) count for that retreat.
    pub launch_flares_evade_range: f32,
    /// Engine boost while that retreat runs (`1.0` = none, `2.0` = like the player's Shift).
    ///
    /// Limited to `1.0` through `4.0`.
    pub launch_flares_boost: f32,
    /// A ship only drops flares in a curtain while its racks hold at least this share of the most flares it was seen
    /// to carry (so some are left for a real panic). Below it, or with no flare launcher at all, the curtain only
    /// retreats. `0.0` = always drop.
    ///
    /// Limited to `0.0` through `1.0`.
    pub launch_flares_min_rack: f32,
    /// Preventive defence only: a curtain starts only for the ship the launched missile is aimed at, or a ship within
    /// `launch_flares_neighbor_range` of it. `false` = every enemy ship rolls (the v0.5.0 behaviour).
    pub launch_flares_targeted_only: bool,
    /// How close (game units) a ship must be to the targeted ship to join its curtain. `0` = only the targeted ship.
    ///
    /// Limited to `0` through `5000`.
    pub launch_flares_neighbor_range: f32,
    /// A curtain ship that has no flares at all (no launcher) heads for an ally that is dropping flares (so the flares
    /// sit between the missile and it) instead of just running away from the missile.
    pub launch_flares_shelter: bool,
    /// Only allies closer than this (game units) count as shelter.
    ///
    /// Limited to `0` through `5000`.
    pub launch_flares_shelter_range: f32,
    /// A curtain waits a random time in this range (seconds) after the launch before its first drop, so the flares
    /// are still in the air when the missile arrives (they disappear after a short time).
    ///
    /// Limited to `0` through `30`.
    pub launch_flares_delay_min: f32,
    pub launch_flares_delay_max: f32,
    /// With a known missile ETA (distance / `launch_flares_missile_speed`) the first drop is `ETA - launch_flares_eta_lead`
    /// seconds after the launch, never later than `launch_flares_eta_max_wait`. Flares live 5 s, so a lead near 4 puts
    /// the cloud in the air when the missile arrives.
    pub launch_flares_eta_lead: f32,
    pub launch_flares_eta_max_wait: f32,
    pub launch_flares_missile_speed: f32,
    /// Chance for the ship the missile is aimed at (the neighbours use `launch_flares_probability`).
    pub launch_flares_target_probability: f32,
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
            launch_flares_probability: 0.6,
            launch_flares_interval_min: 0.5,
            launch_flares_interval_max: 1.0,
            launch_flares_seconds: 5.0,
            launch_flares_max_drops: 6,
            launch_flares_cooldown: 6.0,
            launch_flares_evade: 0.9,
            launch_flares_evade_range: 3000.0,
            launch_flares_boost: 2.0,
            launch_flares_min_rack: 0.5,
            launch_flares_targeted_only: true,
            launch_flares_neighbor_range: 600.0,
            launch_flares_shelter: true,
            launch_flares_shelter_range: 1200.0,
            launch_flares_delay_min: 1.0,
            launch_flares_delay_max: 2.0,
            launch_flares_eta_lead: 4.0,
            launch_flares_eta_max_wait: 8.0,
            launch_flares_missile_speed: 230.0,
            launch_flares_target_probability: 1.0,
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
            launch_flares_probability: pick(
                self.launch_flares_probability,
                default.launch_flares_probability,
                0.0,
                1.0,
            ),
            launch_flares_interval_min: pick(
                self.launch_flares_interval_min,
                default.launch_flares_interval_min,
                0.05,
                MAX_SECONDS,
            ),
            launch_flares_interval_max: pick(
                self.launch_flares_interval_max,
                default.launch_flares_interval_max,
                0.05,
                MAX_SECONDS,
            )
            .max(pick(self.launch_flares_interval_min, default.launch_flares_interval_min, 0.05, MAX_SECONDS)),
            launch_flares_seconds: pick(self.launch_flares_seconds, default.launch_flares_seconds, 0.1, MAX_SECONDS),
            launch_flares_max_drops: self.launch_flares_max_drops.min(100),
            launch_flares_cooldown: pick(self.launch_flares_cooldown, default.launch_flares_cooldown, 0.0, MAX_SECONDS),
            launch_flares_boost: pick(self.launch_flares_boost, default.launch_flares_boost, 1.0, MAX_PANIC_BOOST),
            launch_flares_min_rack: pick(self.launch_flares_min_rack, default.launch_flares_min_rack, 0.0, 1.0),
            launch_flares_targeted_only: self.launch_flares_targeted_only,
            launch_flares_neighbor_range: pick(self.launch_flares_neighbor_range, default.launch_flares_neighbor_range, 0.0, MAX_TRIGGER_RANGE),
            launch_flares_shelter: self.launch_flares_shelter,
            launch_flares_shelter_range: pick(self.launch_flares_shelter_range, default.launch_flares_shelter_range, 0.0, MAX_TRIGGER_RANGE),
            launch_flares_delay_min: pick(self.launch_flares_delay_min, default.launch_flares_delay_min, 0.0, 30.0),
            launch_flares_delay_max: pick(self.launch_flares_delay_max, default.launch_flares_delay_max, 0.0, 30.0)
                .max(pick(self.launch_flares_delay_min, default.launch_flares_delay_min, 0.0, 30.0)),
            launch_flares_eta_lead: pick(self.launch_flares_eta_lead, default.launch_flares_eta_lead, 0.0, 10.0),
            launch_flares_eta_max_wait: pick(self.launch_flares_eta_max_wait, default.launch_flares_eta_max_wait, 0.0, 30.0),
            launch_flares_missile_speed: pick(self.launch_flares_missile_speed, default.launch_flares_missile_speed, 50.0, 1000.0),
            launch_flares_target_probability: pick(self.launch_flares_target_probability, default.launch_flares_target_probability, 0.0, 1.0),
            stray_missiles: self.stray_missiles,
            stray_missile_radius: pick(self.stray_missile_radius, default.stray_missile_radius, 10.0, 1000.0),
            stray_missile_eta_s: pick(self.stray_missile_eta_s, default.stray_missile_eta_s, 0.2, 20.0),
            stray_missile_range: pick(self.stray_missile_range, default.stray_missile_range, 100.0, MAX_TRIGGER_RANGE * 2.0),
            launch_flares_evade: pick(self.launch_flares_evade, default.launch_flares_evade, 0.0, 1.0),
            launch_flares_evade_range: pick(self.launch_flares_evade_range, default.launch_flares_evade_range, 0.0, MAX_TRIGGER_RANGE * 2.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_fields_use_defaults() {
        let config: AiFlaresConfig = serde_json::from_str("{}").unwrap();

        assert!(config.enabled);
        assert_eq!(config.trigger_range, 400.0);
        assert_eq!(config.burst_seconds, 1.5);
        assert!(config.evade);
        assert!(config.allies);
        assert!(!config.log_panic);
        assert_eq!(config.panic_evasion_probability, 0.1);
        assert_eq!(config.panic_evasion_seconds, 5.0);
        assert_eq!(config.barrel_roll_seconds, 2.0);
    }

    #[test]
    fn partial_section_keeps_other_defaults() {
        let config: AiFlaresConfig =
            serde_json::from_str(r#"{"enabled":true,"trigger_range":900.0}"#).unwrap();

        assert!(config.enabled);
        assert_eq!(config.trigger_range, 900.0);
        assert_eq!(config.min_interval_seconds, 1.5);
        assert_eq!(config.barrel_roll_probability, 0.001);
    }

    #[test]
    fn values_are_clamped() {
        let config = AiFlaresConfig {
            enabled: true,
            trigger_range: 1.0e9,
            reaction_probability: 7.0,
            burst_seconds: 0.0,
            min_interval_seconds: f32::NAN,
            evade: true,
            allies: false,
            log_panic: false,
            evasion_seconds: 1.0e9,
            evasion_strength: -2.0,
            barrel_roll_probability: 5.0,
            barrel_roll_seconds: 0.0,
            panic_evasion_probability: 3.0,
            panic_boost: 0.0,
            panic_evasion_seconds: 1.0e9,
            ..AiFlaresConfig::default()
        }
        .sanitized();

        assert_eq!(config.trigger_range, MAX_TRIGGER_RANGE);
        assert_eq!(config.reaction_probability, 1.0);
        assert_eq!(config.burst_seconds, 0.05);
        assert_eq!(config.min_interval_seconds, 1.5);
        assert_eq!(config.evasion_seconds, MAX_SECONDS);
        assert_eq!(config.evasion_strength, 0.0);
        assert_eq!(config.barrel_roll_probability, 1.0);
        assert_eq!(config.panic_evasion_probability, 1.0);
        assert_eq!(config.panic_boost, 1.0);
        assert_eq!(config.panic_evasion_seconds, MAX_SECONDS);
    }

    #[test]
    fn launch_flares_player_defaults_and_sanitize() {
        let config: AiFlaresConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config.launch_flares_probability, 0.6);
        let c = AiFlaresConfig {
            launch_flares_probability: 3.0,
            launch_flares_interval_min: 2.0,
            launch_flares_interval_max: 0.1,
            launch_flares_max_drops: 10_000,
            ..AiFlaresConfig::default()
        }
        .sanitized();
        assert_eq!(c.launch_flares_probability, 1.0);
        assert!(c.launch_flares_interval_max >= c.launch_flares_interval_min);
        assert_eq!(c.launch_flares_max_drops, 100);
    }
}
