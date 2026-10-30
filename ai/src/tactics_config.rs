use serde::{Deserialize, Serialize};

use crate::tactics_math::Params;

/// The `tactics` block of `hf_ai.json` ("strafe in the band", see README). Off by default; every key optional.
/// Distances are game units, times are game seconds. Applies to AI-flown, airborne ships with a living target only.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TacticsConfig {
    /// Master switch (A/B tests).
    pub enabled: bool,
    /// Also steer AI ships of the player's side (allies). The ship you fly is never touched.
    #[serde(rename = "allies", alias = "apply_to_allies")]
    pub apply_to_allies: bool,
    /// Band centre = `k_center` x own gun reach, limited to `band_min..band_max`.
    pub k_center: f32,
    pub band_min: f32,
    pub band_max: f32,
    /// Half the band width: strafe between centre - half_width and centre + half_width.
    pub half_width: f32,
    /// Extra distance when the target's gun reach is longer than ours.
    pub outrange_extra: f32,
    /// Weight of the tactic direction against the unit stock direction (0 = stock).
    pub blend_weight: f32,
    /// The strafe side reverses after a random time in this range.
    pub strafe_min_s: f32,
    pub strafe_max_s: f32,
    /// Tangential share in the too-close retreat (0 = straight back).
    pub retreat_tangent: f32,
    /// Ground reference y (y grows downward) and the clearance above it where no downward component is added.
    pub ground_y: f32,
    pub ground_clearance: f32,
    /// Below this y no tactic steers downward at all (the ground is about +250; y grows downward).
    pub ground_hard_y: f32,
    /// No outward component within this distance of an arena edge.
    pub edge_margin: f32,
    /// "Strafe in the band" on/off (`enabled` stays the master switch).
    pub strafe_enabled: bool,
    /// "Deny the dive": sidestep when the target hangs straight below / above. Own switch, works with the strafe off.
    pub dive_enabled: bool,
    /// Chance (0..1) that the sidestep starts; rolled once each time the target enters the cone.
    pub dive_chance: f32,
    /// Half-angle in degrees around straight down / up.
    pub dive_cone_deg: f32,
    pub dive_range: f32,
    /// The target must stay in the cone this long before the roll.
    pub dive_dwell_s: f32,
    pub dive_min_s: f32,
    pub dive_max_s: f32,
    pub dive_cooldown_s: f32,
    pub dive_weight: f32,
    /// React to a target below us and / or above us.
    pub dive_under: bool,
    pub dive_over: bool,
    /// Engine boost while dodging (1.0 = off, 2.0 = like Shift). Needs `ai_flares.enabled` (its engine hook carries it).
    pub dive_boost: f32,
    /// Wrong-foot speed (u/s): a target moving sideways faster than this makes the dodge go the other way (0 = random side).
    pub dive_wrongfoot_speed: f32,
    /// Own chance and dwell for a target BELOW us (the damaging case); `dive_chance` / `dive_dwell_s` apply to a target above.
    pub dive_under_chance: f32,
    pub dive_under_dwell_s: f32,
    /// Vertical pull onto the target's altitude mixed into the dodge (0 = purely sideways).
    pub dive_level_weight: f32,
    /// "Aim awareness" (off by default): in shared awareness windows the enemies that target the player see the
    /// player's aim line and slip off it. Window length, random time from the start of one window to the start of the next, hit tolerance
    /// (miss distance from the aim ray), range along the ray, sidestep weight and engine boost.
    pub aim_enabled: bool,
    pub aim_window_s: f32,
    pub aim_gap_min_s: f32,
    pub aim_gap_max_s: f32,
    pub aim_tolerance: f32,
    pub aim_range: f32,
    pub aim_weight: f32,
    pub aim_boost: f32,
    /// The aim dodge only happens while the player is actually shooting (either gun group's fire byte set).
    pub aim_require_fire: bool,
    /// Early fire (0 = off): chance, rolled once per awareness window for a ship on the aim line, that it opens fire
    /// with its main guns for `aim_fire_s` seconds while the player is within `aim_fire_range` x its gun reach.
    pub aim_fire_probability: f32,
    pub aim_fire_s: f32,
    pub aim_fire_range: f32,
}

impl Default for TacticsConfig {
    fn default() -> Self {
        let p = Params::default();
        Self {
            enabled: false,
            apply_to_allies: p.apply_to_allies,
            k_center: p.k_center,
            band_min: p.band_min,
            band_max: p.band_max,
            half_width: p.half_width,
            outrange_extra: p.outrange_extra,
            blend_weight: p.blend_weight,
            strafe_min_s: p.strafe_min_s,
            strafe_max_s: p.strafe_max_s,
            retreat_tangent: p.retreat_tangent,
            ground_y: p.ground_y,
            ground_clearance: p.ground_clearance,
            ground_hard_y: p.ground_hard_y,
            edge_margin: p.edge_margin,
            strafe_enabled: p.strafe_enabled,
            dive_enabled: p.dive_enabled,
            dive_chance: p.dive_chance,
            dive_cone_deg: p.dive_cone_deg,
            dive_range: p.dive_range,
            dive_dwell_s: p.dive_dwell_s,
            dive_min_s: p.dive_min_s,
            dive_max_s: p.dive_max_s,
            dive_cooldown_s: p.dive_cooldown_s,
            dive_weight: p.dive_weight,
            dive_under: p.dive_under,
            dive_over: p.dive_over,
            dive_boost: p.dive_boost,
            dive_wrongfoot_speed: p.dive_wrongfoot_speed,
            dive_under_chance: p.dive_under_chance,
            dive_under_dwell_s: p.dive_under_dwell_s,
            dive_level_weight: p.dive_level_weight,
            aim_enabled: p.aim_enabled,
            aim_window_s: p.aim_window_s,
            aim_gap_min_s: p.aim_gap_min_s,
            aim_gap_max_s: p.aim_gap_max_s,
            aim_tolerance: p.aim_tolerance,
            aim_range: p.aim_range,
            aim_weight: p.aim_weight,
            aim_boost: p.aim_boost,
            aim_require_fire: p.aim_require_fire,
            aim_fire_probability: p.aim_fire_probability,
            aim_fire_s: p.aim_fire_s,
            aim_fire_range: p.aim_fire_range,
        }
    }
}

impl TacticsConfig {
    /// Every value limited to its accepted range; non-finite numbers fall back to the default.
    pub fn sanitized(&self) -> Self {
        let d = Self::default();
        let pick = |v: f32, fallback: f32, lo: f32, hi: f32| if v.is_finite() { v.clamp(lo, hi) } else { fallback };
        let band_min = pick(self.band_min, d.band_min, 100.0, 3000.0);
        let strafe_min_s = pick(self.strafe_min_s, d.strafe_min_s, 0.3, 60.0);
        let dive_min_s = pick(self.dive_min_s, d.dive_min_s, 0.2, 30.0);
        let aim_window_s = pick(self.aim_window_s, d.aim_window_s, 0.1, 30.0);
        let aim_gap_min_s = pick(self.aim_gap_min_s, d.aim_gap_min_s, aim_window_s, 120.0);
        Self {
            enabled: self.enabled,
            apply_to_allies: self.apply_to_allies,
            k_center: pick(self.k_center, d.k_center, 0.05, 1.0),
            band_min,
            band_max: pick(self.band_max, d.band_max, band_min, 4000.0),
            half_width: pick(self.half_width, d.half_width, 20.0, 1000.0),
            outrange_extra: pick(self.outrange_extra, d.outrange_extra, 0.0, 1500.0),
            blend_weight: pick(self.blend_weight, d.blend_weight, 0.0, 4.0),
            strafe_min_s,
            strafe_max_s: pick(self.strafe_max_s, d.strafe_max_s, strafe_min_s, 120.0),
            retreat_tangent: pick(self.retreat_tangent, d.retreat_tangent, 0.0, 3.0),
            ground_y: pick(self.ground_y, d.ground_y, -5000.0, 5000.0),
            ground_clearance: pick(self.ground_clearance, d.ground_clearance, 0.0, 5000.0),
            ground_hard_y: pick(self.ground_hard_y, d.ground_hard_y, -5000.0, 5000.0),
            edge_margin: pick(self.edge_margin, d.edge_margin, 0.0, 3000.0),
            strafe_enabled: self.strafe_enabled,
            dive_enabled: self.dive_enabled,
            dive_chance: pick(self.dive_chance, d.dive_chance, 0.0, 1.0),
            dive_cone_deg: pick(self.dive_cone_deg, d.dive_cone_deg, 5.0, 85.0),
            dive_range: pick(self.dive_range, d.dive_range, 100.0, 3000.0),
            dive_dwell_s: pick(self.dive_dwell_s, d.dive_dwell_s, 0.0, 30.0),
            dive_min_s,
            dive_max_s: pick(self.dive_max_s, d.dive_max_s, dive_min_s, 60.0),
            dive_cooldown_s: pick(self.dive_cooldown_s, d.dive_cooldown_s, 0.0, 120.0),
            dive_weight: pick(self.dive_weight, d.dive_weight, 0.0, 20.0),
            dive_under: self.dive_under,
            dive_over: self.dive_over,
            dive_boost: pick(self.dive_boost, d.dive_boost, 1.0, 4.0),
            dive_wrongfoot_speed: pick(self.dive_wrongfoot_speed, d.dive_wrongfoot_speed, 0.0, 1000.0),
            dive_under_chance: pick(self.dive_under_chance, d.dive_under_chance, 0.0, 1.0),
            dive_under_dwell_s: pick(self.dive_under_dwell_s, d.dive_under_dwell_s, 0.0, 30.0),
            dive_level_weight: pick(self.dive_level_weight, d.dive_level_weight, 0.0, 3.0),
            aim_enabled: self.aim_enabled,
            aim_window_s,
            aim_gap_min_s,
            aim_gap_max_s: pick(self.aim_gap_max_s, d.aim_gap_max_s, aim_gap_min_s, 300.0),
            aim_tolerance: pick(self.aim_tolerance, d.aim_tolerance, 10.0, 2000.0),
            aim_range: pick(self.aim_range, d.aim_range, 100.0, 5000.0),
            aim_weight: pick(self.aim_weight, d.aim_weight, 0.0, 20.0),
            aim_boost: pick(self.aim_boost, d.aim_boost, 1.0, 4.0),
            aim_require_fire: self.aim_require_fire,
            aim_fire_probability: pick(self.aim_fire_probability, d.aim_fire_probability, 0.0, 1.0),
            aim_fire_s: pick(self.aim_fire_s, d.aim_fire_s, 0.1, 10.0),
            aim_fire_range: pick(self.aim_fire_range, d.aim_fire_range, 0.5, 4.0),
        }
    }

    pub fn params(&self) -> Params {
        let s = self.sanitized();
        Params {
            enabled: s.enabled,
            apply_to_allies: s.apply_to_allies,
            k_center: s.k_center,
            band_min: s.band_min,
            band_max: s.band_max,
            half_width: s.half_width,
            outrange_extra: s.outrange_extra,
            blend_weight: s.blend_weight,
            strafe_min_s: s.strafe_min_s,
            strafe_max_s: s.strafe_max_s,
            retreat_tangent: s.retreat_tangent,
            ground_y: s.ground_y,
            ground_clearance: s.ground_clearance,
            ground_hard_y: s.ground_hard_y,
            edge_margin: s.edge_margin,
            strafe_enabled: s.strafe_enabled,
            dive_enabled: s.dive_enabled,
            dive_chance: s.dive_chance,
            dive_cone_deg: s.dive_cone_deg,
            dive_range: s.dive_range,
            dive_dwell_s: s.dive_dwell_s,
            dive_min_s: s.dive_min_s,
            dive_max_s: s.dive_max_s,
            dive_cooldown_s: s.dive_cooldown_s,
            dive_weight: s.dive_weight,
            dive_under: s.dive_under,
            dive_over: s.dive_over,
            dive_boost: s.dive_boost,
            dive_wrongfoot_speed: s.dive_wrongfoot_speed,
            dive_under_chance: s.dive_under_chance,
            dive_under_dwell_s: s.dive_under_dwell_s,
            dive_level_weight: s.dive_level_weight,
            aim_enabled: s.aim_enabled,
            aim_window_s: s.aim_window_s,
            aim_gap_min_s: s.aim_gap_min_s,
            aim_gap_max_s: s.aim_gap_max_s,
            aim_tolerance: s.aim_tolerance,
            aim_range: s.aim_range,
            aim_weight: s.aim_weight,
            aim_boost: s.aim_boost,
            aim_require_fire: s.aim_require_fire,
            aim_fire_probability: s.aim_fire_probability,
            aim_fire_s: s.aim_fire_s,
            aim_fire_range: s.aim_fire_range,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_off_and_match_the_math_defaults() {
        let c: TacticsConfig = serde_json::from_str("{}").unwrap();
        assert!(!c.enabled && !c.apply_to_allies);
        assert_eq!(c.k_center, 0.32);
        let p = c.params();
        assert_eq!((p.band_min, p.band_max, p.half_width), (300.0, 550.0, 100.0));
        let partial: TacticsConfig = serde_json::from_str(r#"{"enabled": true, "half_width": 80}"#).unwrap();
        assert!(partial.enabled && partial.half_width == 80.0 && partial.strafe_max_s == 5.0);
    }

    #[test]
    fn allies_key_and_legacy_alias_preserve_the_allied_steering_gate() {
        use crate::tactics_math::{gate, Gate, GateInput};

        let input = GateInput {
            ai_flag: 1, side: 1.0, mode: Some(1), has_target: true, panicking: false,
        };
        for key in ["allies", "apply_to_allies"] {
            for enabled in [false, true] {
                let text = format!(r#"{{"enabled":true,"{key}":{enabled}}}"#);
                let config: TacticsConfig = serde_json::from_str(&text).unwrap();
                assert_eq!(config.apply_to_allies, enabled);
                assert_eq!(gate(&config.params(), &input), if enabled { Gate::Pass } else { Gate::Stock });
                let json = serde_json::to_value(config.sanitized()).unwrap();
                assert_eq!(json["allies"], enabled);
                assert!(json.get("apply_to_allies").is_none());
            }
        }
    }

    #[test]
    fn retired_acceleration_keys_are_ignored_and_not_serialized() {
        let config: TacticsConfig = serde_json::from_str(
            r#"{"enabled":true,"aim_enabled":true,"min_accel":999999,"dive_min_accel":999999}"#,
        ).unwrap();
        let params = config.params();
        assert!(params.enabled && params.strafe_enabled && params.dive_enabled && params.aim_enabled);
        let json = serde_json::to_value(&config.sanitized()).unwrap();
        assert!(json.get("min_accel").is_none());
        assert!(json.get("dive_min_accel").is_none());
    }

    #[test]
    fn legacy_acceleration_limits_do_not_disable_any_maneuver() {
        use crate::tactics_math::{self as tm, AimInput, Bounds, Rng, ShipState, State, StepInput};

        let config: TacticsConfig = serde_json::from_str(
            r#"{"enabled":true,"min_accel":999999,"dive_min_accel":999999,
                "aim_enabled":true,"dive_under_dwell_s":0.0,"dive_under_chance":1.0}"#,
        ).unwrap();
        let params = config.params();
        assert!(crate::twr_math::allows(Some(2.0), 2.0));
        assert!(!crate::twr_math::allows(Some(1.99), 2.0));
        let bounds = Bounds { xmin: -1500.0, xmax: 1500.0, ymin: -2000.0 };
        let scenarios = [
            (State::Band, (0.0, -1000.0), (480.0, -1000.0), None),
            (State::Close, (0.0, -1000.0), (250.0, -1000.0), None),
            (State::Dive, (0.0, -1000.0), (0.0, -700.0), None),
            (State::Aim, (0.0, -950.0), (-1000.0, -1000.0),
                Some(AimInput { player: (-1000.0, -1000.0), dir: (1.0, 0.0) })),
        ];
        for (expected, ship, target, aim) in scenarios {
            let mut rng = Rng::new(3);
            let mut state = ShipState::new(0.0, &mut rng, &params);
            state.latch_reach(Some(1500.0), 1, Some(1500.0));
            let input = StepInput { now: 1.0, ship, target, aim, wanted: (ship.0 + 200.0, ship.1) };
            let decision = tm::step(&params, &mut state, &bounds, &input, &mut rng);
            assert_eq!(decision.state, expected);
            assert!(decision.waypoint.is_some(), "{expected:?} must still steer");
        }
    }

    #[test]
    fn values_are_sanitized() {
        let c = TacticsConfig {
            enabled: true,
            k_center: f32::NAN,
            band_min: 5000.0,
            band_max: 10.0,
            half_width: -3.0,
            blend_weight: 1.0e9,
            strafe_min_s: 0.0,
            strafe_max_s: -1.0,
            ..TacticsConfig::default()
        }
        .sanitized();
        assert_eq!(c.k_center, 0.32);
        assert_eq!(c.band_min, 3000.0);
        assert!(c.band_max >= c.band_min);
        assert_eq!(c.half_width, 20.0);
        assert_eq!(c.blend_weight, 4.0);
        assert_eq!(c.strafe_min_s, 0.3);
        assert!(c.strafe_max_s >= c.strafe_min_s);
    }

    #[test]
    fn dive_defaults_and_sanitizing() {
        let c: TacticsConfig = serde_json::from_str("{}").unwrap();
        assert!(c.dive_enabled && c.strafe_enabled && c.dive_chance == 0.8 && c.dive_cone_deg == 45.0);
        let bad = TacticsConfig { dive_chance: 7.0, dive_cone_deg: f32::NAN, dive_min_s: 5.0, dive_max_s: 1.0, ..TacticsConfig::default() }.sanitized();
        assert_eq!((bad.dive_chance, bad.dive_cone_deg), (1.0, 45.0));
        assert!(bad.dive_max_s >= bad.dive_min_s);
        let off: TacticsConfig = serde_json::from_str(r#"{"strafe_enabled": false, "dive_chance": 0.5}"#).unwrap();
        assert!(!off.params().strafe_enabled && off.params().dive_chance == 0.5);
    }
}
