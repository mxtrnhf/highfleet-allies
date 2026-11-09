# hf_ai settings (`Modloader/config/hf_ai.json`)

The checked-in [player preset](../ai/config/hf_ai.json) and compiled defaults use the same values (hf_ai 1.0.0; the preset dropped early-fire keys in 0.10.19 and changed the default `dive_dwell_s` to 0.3 in 0.10.20; curtain keys were removed in 0.10.18). The plain-language explanation of how the AI works and what to turn for which effect is in [hf-ai-tuning-guide.md](hf-ai-tuning-guide.md); this file is the complete key list with the code defaults. Edit while the game is closed; `"enabled": false` turns hf_ai off. Older release packs retain their original DLLs.

## Missing keys and how to revert to vanilla

**You do not have to list every key.** Every top-level key, nested block and key inside a block may be omitted; hf_ai supplies the code default shown below. Since v0.10.11, the shipped `hf_ai.json` has been trimmed repeatedly; the current v1.1.0 preset lists 39 gameplay settings plus `logging.log_panic` (40 total leaf settings). Internal timing/math parameters use their compiled defaults unless explicitly added. An empty `{}` is also valid. If the file is absent, hf_ai tries to create it with the full defaults; if the JSON is invalid, it logs an error and uses the defaults.

**Player default does not mean vanilla.** Missing keys use the active player profile; flares, tactics and missile dumping are enabled by default. Use the explicit values below when you want stock behaviour. There is no general `-1` disable convention.

For the most complete revert, use only:

```json
{ "enabled": false }
```

To keep hf_ai loaded but restore stock gameplay and disable its optional data files, use:

```json
{
  "enabled": true,
  "logging": { "log_panic": false },
  "plane_group_per_launch": 1,
  "ai_missile_dump_chance": 0.0,
  "ai_flares": { "enabled": false },
  "tactics": { "enabled": false }
}
```

This leaves hf_ai's pass-through steering hook installed; `"enabled": false` avoids that too. QoL has independent flare and missile-dump settings, so revert those separately if they are enabled.

### Feature-by-feature vanilla settings

| Feature | Setting that disables or neutralizes it | What remains / important dependency |
|---|---|---|
| Entire mod | `"enabled": false` | No hf_ai gameplay hooks are installed. |
| All flare and panic behaviour | `ai_flares.enabled: false` | Tactics, tactics engine boosts, plane groups and missile dump remain. |
| All threat-triggered panics, locked and stray | `ai_flares.reaction_probability: 0.0` | No new threat panic, flare burst, roll or panic evasion. |
| Locked-missile flare bursts only, while keeping locked evasion | **No independent off switch** | `burst_seconds: 0.0` is clamped to `0.05`, not off. Use `reaction_probability: 0.0` to stop all threat panics, or disable the whole flare subsystem. |
| All flare-related manoeuvres | `ai_flares.evade: false` | Locked-missile flare bursts still run; plain/panic evasion and rolls stop. |
| Plain evasion steering only | `ai_flares.evasion_strength: 0.0` | Neutralizes its steering contribution, but the panic state, boosted panic evasion and rolls remain possible. |
| Stray/lost-lock missile evasion | `ai_flares.stray_missiles: false` | Locked-missile behaviour remains. |
| Allied flares and panic | `ai_flares.allies: false` | Enemy behaviour remains. The player-flown ship is always excluded. |
| Allied tactical steering and aim dodging | `tactics.allies: false` | Independent of `ai_flares.allies`; set both to false to exclude allies from everything, including allied missile dumping. |
| Barrel rolls | `ai_flares.barrel_roll_probability: 0.0` | Plain and boosted panic evasion remain. |
| Boosted/full-steering panic evasion | `ai_flares.panic_evasion_probability: 0.0` | Plain evasion and barrel rolls remain. |
| Panic/roll boost and dash | `ai_flares.panic_boost: 1.0` | Steering and roll angles remain. |
| Both dive directions | `tactics.dive_enabled: false` | Aim awareness remains. |
| Dive-under only | `tactics.dive_under: false` | Dive-over remains. |
| Dive-over only | `tactics.dive_over: false` | Dive-under remains. |
| Dive engine boost and dash | `tactics.dive_boost: 1.0` | Dive steering remains. Works independently of `ai_flares.enabled`. |
| Aim awareness (dodging hostile gun aim) | `tactics.aim_enabled: false` | No aim-ray steering. Dive avoidance and missile defence remain. |
| Aim engine boost and dash | `tactics.aim_boost: 1.0` | Aim steering remains. Works independently of `ai_flares.enabled`. |
| Tactical carrier plane groups | `plane_group_per_launch: 1` | Restores stock one-at-a-time cadence; strategic and manual launch paths are unchanged. |
| Zenith/missile dumping | `ai_missile_dump_chance: 0.0` | Restores vanilla launch timing for enemies and allies. Delay values do not disable dumping. |
| Panic trace | `logging.log_panic: false` | General startup and diagnostic messages in `Modloader.log` are separate. |

## Logging

All logging options live in `"logging"`. `log_panic` defaults to false and writes
`Modloader/logs/hf_ai_panic_<unix>.log`: text panic events with game time and
ship pointer. The old `ai_flares.log_panic` location remains readable; an
explicit `logging.log_panic` wins. Ship/steering/obstacle CSV capture has been
removed; old `log_steering`, `log_hz`, `log_scan` and `log_scan_bodies` keys do nothing.

## Top level

| Key | Default | If key omitted | What it does |
|---|---|---|---|
| `enabled` | `true` | Uses player default `true`. | Master switch. |
| `ally_tick_skip` | `true` | Uses player default `true`. | Skips the whole native AI tick of every player-side AI ship (kind 1, side above zero; wingmen, strike-group ships, and the flagship you switched away from) on every other 20 ms frame, by wrapping the `aiShipDriver` call in the `SomethingAITask` loop. Halves the ally share of the quadratic per-tick obstacle scan in ally-heavy battles. Trade-offs: ally commands run 20 ms stale, and the native per-tick launch/re-arm timers (6.0 s aircraft `+0xcb8`, 2.0 s Zenith `+0xd60`, 0.5 s AA `+0xd64`) run at half cadence. hf_ai's own tactic layer is game-clock based and stays time-correct. hf_perf's `ai_driver` span needs the same call site and cannot install while this is on; set `false` for an attribution capture. Restart to apply. |
| `twr_cutoff` | `2.0` | Uses player default `2.0`. | Minimum live strategic/Shipworks TWR (`creature +0x160c`, before UI rounding) for tactics, missile evasion, rolls/boosts and stray-missile reactions. Below the cutoff these are suppressed; at or above it their individual switches still apply. Locked-missile flare bursts remain available. Negative/non-finite cutoffs use `0.0`. Independent of carrier plane groups. Restart after editing. |
| `plane_group_per_launch` | `3` | Uses player default `3`. | Clamped to `1..64`; `1` restores stock cadence. Enemy and allied AI, normal `Combat.seria` only. Stock deck/availability checks remain; blocked attempts cancel a partial group. Independent of TWR, tactics and flares. |
| `plane_group_launch_delay_seconds` | `0.5` | Uses player default `0.5`. | Clamped to `0.05..60` seconds; non-finite values use `0.5`. Stock six-second cooldown returns after the last member. Restart to apply. |
| `ai_missile_dump_chance` | `0.4` | Uses player default `0.4`. | Chance (`0..1`) that an AI ship (enemy, or ally when `ai_flares.allies` or `tactics.allies` is true) keeps launching Zenith missiles right after a launch instead of waiting for the game's 2 s cooldown. `0` = vanilla. |
| `ai_missile_dump_min_delay` | `0.6` | Uses player default `0.6`. | Shortest wait between the launches of a burst, in seconds. |
| `ai_missile_dump_max_delay` | `1.5` | Uses player default `1.5`. | Longest wait between the launches of a burst, in seconds (`max` never below `min`). |

## `ai_flares` (flares, evasion, panic)

The preventive curtain subsystem was retired in **v0.10.18**. Old `curtain_*`
and `launch_flares_*` keys are ignored by the parser and do nothing.

| Key | Default | If key omitted | What it does |
|---|---|---|---|
| `enabled` | `true` | Uses player default `true`. | Turns AI missile defence on. Enable it only when qol's `ai_flares.enabled` is `false`: both patch the same sites. |
| `allies` | `true` | Uses player default `true`. | Lets AI-run ships of the player's own side (allies/wingmen) use flares and missile evasion too. The ship you fly yourself is never affected. |
| `trigger_range` | `400.0` | Uses player default `400.0`. | How close, in game units, an incoming locked missile has to be before the ship reacts (`0..5000`). |
| `reaction_probability` | `0.6` | Uses player default `0.6`. | Chance that the ship reacts to a given missile (`0.0..1.0`). Rolled once when that missile first enters `trigger_range`; a failed roll ignores that missile body until the immediate threat picture clears. Other missiles, and later approaches after the picture clears, roll separately. |
| `burst_seconds` | `1.5` | Uses player default `1.5`. | How long, in seconds, the launchers keep dropping flares once the panic starts (`0.05..60`). |
| `min_interval_seconds` | `1.5` | Uses player default `1.5`. | Quiet time, in seconds, after a panic ends before the same ship may panic again (`0..60`). |
| `evade` | `true` | Uses player default `true`. | Makes the ship also manoeuvre while it panics. With `false` it only drops flares. Ships without flare launchers still evade when `evade` is `true`. |
| `evasion_seconds` | `3.0` | Uses player default `3.0`. | How long, in seconds, the evasive manoeuvre lasts, extended by 1 s while a locked missile stays in range (`0.05..60`). |
| `evasion_strength` | `0.5` | Uses player default `0.5`. | How much of the ship's course plain evasion takes over (`0.0..1.0`). |
| `panic_evasion_probability` | `0.1` | Uses player default `0.1`. | Chance, rolled once when a panic starts, that the evasion is a full-takeover dashing *panic evasion* (`0.0..1.0`). |
| `panic_evasion_seconds` | `5.0` | Uses player default `5.0`. | Duration of a panic evasion in seconds, extended by 2.5 s at a time (`0.05..60`). |
| `panic_boost` | `2.0` | Uses player default `2.0`. | Engine strength multiplier during a panic evasion or barrel roll (`1.0..4.0`). Above `1.0` a panic evasion also dashes along its committed escape direction (both engine banks and swivel servos aimed there); `1.0` disables the strength raise and the dash. |
| `barrel_roll_probability` | `0.001` | Uses player default `0.001`. | Chance, rolled once when a panic starts, that the ship does a panic spin instead of evading (`0.0..1.0`). |
| `barrel_roll_seconds` | `2.0` | Uses player default `2.0`. | Duration of a barrel roll in seconds (`0.05..60`). |
| `stray_missiles` | `true` | Uses player default `true`. | Reacts (evasion only, no flare burst) when an unguided/other-target missile's trajectory passes close to the ship. |

## `tactics` (universal aim dodging and dive avoidance)

| Key | Default | If key omitted | What it does |
|---|---|---|---|
| `enabled` | `true` | Uses player default `true`. | Master switch for the `tactics` block. |
| `allies` | `true` | Uses player default `true`. | Also applies dive avoidance and aim dodging to AI-flown allied ships. The player-flown ship is never touched. |
| `dive_enabled` | `true` | Uses player default `true`. | "Deny the dive": sidestep when the target hangs inside the vertical cone above or below. |
| `dive_under` | `true` | Uses player default `true`. | React to a target hanging below the ship. |
| `dive_over` | `true` | Uses player default `true`. | React to a target hanging above the ship. |
| `dive_chance` | `0.8` | Uses player default `0.8`. | Chance (`0.0..1.0`) that a dive sidestep starts after `dive_dwell_s` in the cone (applies to both `dive_under` and `dive_over`). On a failed roll, the ship waits `dive_cooldown_s` and re-evaluates. |
| `dive_range` | `600.0` | Uses player default `600.0`. | Maximum distance (units) at which a target above/below triggers dive avoidance (`100..3000`). |
| `dive_dwell_s` | `0.3` | Uses player default `0.3`. | Seconds the target must stay in the vertical cone before rolling `dive_chance` (`0..30`). |
| `dive_cooldown_s` | `2.0` | Uses player default `2.0`. | Pause in seconds after a dive sidestep (or declined roll) before the next roll (`0..120`). |
| `dive_boost` | `2.0` | Uses player default `2.0`. | Engine boost multiplier while dive-dodging (`1.0..4.0`). Above `1.0` the ship also dashes along the sidestep: both engine banks and the swivel servos are aimed at the dodge direction and floored at this strength, bypassing the stock velocity controller's speed target. `1.0` disables both. Independent of `ai_flares.enabled`. |
| `aim_enabled` | `true` | Uses player default `true`. | Universal aim awareness: during each ship's staggered awareness window, it checks all hostile shooters (player and AI) and slips off the closest active aim ray crossing it. |
| `aim_chance` | `0.75` | Uses player default `0.75`. | Chance (`0.0..1.0`), rolled once per awareness window when a hostile aim ray crosses the ship, that it dodges for that window. Prevents endless synchronized juking in AI-vs-AI fights. |
| `aim_range` | `1800.0` | Uses player default `1800.0`. | Maximum distance (units) along a hostile shooter's aim ray at which a ship reacts (`100..5000`). |
| `aim_tolerance` | `200.0` | Uses player default `200.0`. | Maximum perpendicular miss distance (units) from a hostile aim ray that counts as aiming at the ship (`10..2000`). |
| `aim_boost` | `2.0` | Uses player default `2.0`. | Engine boost multiplier while dodging an aim ray (`1.0..4.0`). Above `1.0` the ship also dashes along the slip direction with both banks and the swivel servos aimed there, bypassing the stock velocity controller's speed target. `1.0` disables both. Independent of `ai_flares.enabled`. |
| `aim_require_fire` | `true` | Uses player default `true`. | Only dodge hostile aim rays from firing shooters. Player button state is sampled at 20 Hz with a 0.6 s hold-over, not confirmed shot detection; brief clicks can be missed. AI detection uses gun rate timers and burst latches. |

---

## Retired settings

### Retired in v0.10.11 and later

The keys below were removed from the default `hf_ai.json` preset in v0.10.11 or later to keep the configuration concise. They fall into two groups:

#### 1. Retired feature keys (ignored if present in JSON)

| Former key | Former default | Status |
|---|---|---|
| `tactics.aim_fire_probability`, `tactics.aim_fire_range`, `tactics.aim_fire_s` | — | Retired in v0.10.19 with offensive early fire; ignored by the parser. |
| `ai_flares.curtain_*` / `launch_flares_*` | — | Retired in v0.10.18 with the preventive flare curtain; ignored by the parser. |
| `ai_flares.launch_flares_shelter` | `true` | Retired. Ships without flares no longer steer behind a flare-dropping ally; they still evade/retreat from the missile directly. |
| `ai_flares.launch_flares_shelter_range` | `1200.0` | Retired with `launch_flares_shelter`. |
| `tactics.dive_under_chance` | `1.0` | Merged into `tactics.dive_chance` (`0.8`). |
| `tactics.dive_under_dwell_s` | `0.5` | Merged into `tactics.dive_dwell_s` (current default `0.3`). |

#### 2. Internal tuning keys (omitted from `hf_ai.json`; still accepted by the parser if added manually)

| Key | Compiled default | What it controls if added to `hf_ai.json` |
|---|---|---|
| `ai_flares.stray_missile_radius` | `150.0` | Miss distance (units, plus half ship radius) for stray-missile collision checks (`10..1000`). |
| `ai_flares.stray_missile_eta_s` | `3.0` | Maximum time to closest approach (s) for stray-missile evasion (`0.2..20`). |
| `ai_flares.stray_missile_range` | `1200.0` | Maximum distance (units) for stray-missile evasion (`100..10000`). |
| `tactics.ground_y` | `200.0` | Ground reference y (y grows downward) (`-5000..5000`). |
| `tactics.ground_clearance` | `450.0` | Clearance above `ground_y` retained across the full tactical waypoint (`0..5000`). Since v0.10.15, at/below the resulting floor, tactical boosts/dashes stop and upward native avoidance is retained. Fixed reference, not terrain sampling. |
| `tactics.ground_hard_y` | `120.0` | Additional hard floor y (`-5000..5000`): the effective tactical floor is `min(ground_y - ground_clearance, ground_hard_y)`. |
| `tactics.edge_margin` | `300.0` | Margin (units) from arena edges within which tactics do not add outward steering (`0..3000`). |
| `tactics.dive_cone_deg` | `45.0` | Half-angle (degrees) around straight up/down for dive detection (`5..85`). |
| `tactics.dive_min_s` | `1.5` | Shortest dive sidestep duration (s) (`0.2..30`). |
| `tactics.dive_max_s` | `3.0` | Longest dive sidestep duration (s) (`dive_min_s..60`). |
| `tactics.dive_weight` | `8.0` | Steering blend weight of the dive sidestep relative to the unit stock direction (`0..20`). |
| `tactics.dive_wrongfoot_speed` | `40.0` | Target horizontal speed (u/s) above which the sidestep goes against the target's motion (`0..1000`). |
| `tactics.dive_level_weight` | `0.7` | Vertical pull toward the airborne target's altitude mixed into the dive sidestep (`0..3`), constrained by the tactical floor. Since v0.10.15 landed/taking-off targets do not trigger dive denial. |
| `tactics.aim_window_s` | `1.5` | Length (s) of each per-ship awareness window (`0.1..30`). |
| `tactics.aim_gap_min_s` | `2.5` | Shortest start-to-start interval (s) between awareness windows (`aim_window_s..120`). |
| `tactics.aim_gap_max_s` | `3.5` | Longest start-to-start interval (s) between awareness windows (`aim_gap_min_s..300`). |
| `tactics.aim_weight` | `12.0` | Steering blend weight of the aim dodge relative to the unit stock direction (`0..20`). |

### Retired in earlier versions (v0.10.4 – v0.10.6)

| Former key | Former default | Current effect | Replacement |
|---|---|---|---|
| `tactics.min_accel` | `35.0` | None (retired in v0.10.4). | Top-level `twr_cutoff` |
| `tactics.dive_min_accel` | `30.0` | None (retired in v0.10.4). | Top-level `twr_cutoff` |
| `tactics.apply_to_allies` | `false` | Accepted as a compatibility alias for `tactics.allies` (renamed in v0.10.5). | `tactics.allies` |
| `tactics.k_center`, `band_min`, `band_max`, `half_width`, `outrange_extra`, `blend_weight`, `strafe_min_s`, `strafe_max_s`, `retreat_tangent`, `strafe_enabled` | various | None; optional positioning routine removed in v0.10.6. | Stock AI positioning |
