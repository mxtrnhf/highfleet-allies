# hf_ai settings (`Modloader/config/hf_ai.json`)

The checked-in [player preset](../ai/config/hf_ai.json) and compiled defaults use the same values (hf_ai 0.10.11). The plain-language explanation of how the AI works and what to turn for which effect is in [hf-ai-tuning-guide.md](hf-ai-tuning-guide.md); this file is the complete key list with the code defaults. Edit while the game is closed; `"enabled": false` turns hf_ai off. Older release packs retain their original DLLs.

## Missing keys and how to revert to vanilla

**You do not have to list every key.** Every top-level key, nested block and key inside a block may be omitted; hf_ai supplies the code default shown below. Since v0.10.11, the shipped `hf_ai.json` lists only the core gameplay knobs (38 keys); internal timing/math parameters use their compiled defaults unless explicitly added. An empty `{}` is also valid. If the file is absent, hf_ai tries to create it with the full defaults; if the JSON is invalid, it logs an error and uses the defaults.

**Player default does not mean vanilla.** Missing keys use the active player profile; flares, tactics, early fire and missile dumping are enabled by default. Use the explicit values below when you want stock behaviour. There is no general `-1` disable convention.

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
| All flare, panic and curtain behaviour | `ai_flares.enabled: false` | Tactics, tactics engine boosts, plane groups and missile dump remain. |
| All threat-triggered panics, locked and stray | `ai_flares.reaction_probability: 0.0` | No new threat panic, flare burst, roll or panic evasion. Preventive curtains are separate and remain if enabled. |
| Locked-missile flare bursts only, while keeping locked evasion | **No independent off switch** | `burst_seconds: 0.0` is clamped to `0.05`, not off. Use `reaction_probability: 0.0` to stop all threat panics, or disable the whole flare subsystem. |
| All flare-related manoeuvres | `ai_flares.evade: false` | Locked-missile flare bursts and curtain drops can still run; plain/panic evasion, rolls and curtain retreat stop. |
| Plain evasion steering only | `ai_flares.evasion_strength: 0.0` | Neutralizes its steering contribution, but the panic state, boosted panic evasion and rolls remain possible. |
| Stray/lost-lock missile evasion | `ai_flares.stray_missiles: false` | Locked-missile behaviour remains. |
| Allied flares, panic and curtains | `ai_flares.allies: false` | Enemy behaviour remains. The player-flown ship is always excluded. |
| Allied tactical steering, aim dodging and early fire | `tactics.allies: false` | Independent of `ai_flares.allies`; set both to false to exclude allies from everything, including allied missile dumping. |
| Preventive flare curtains (all ships) | `ai_flares.launch_flares_probability: 0.0` **and** `ai_flares.launch_flares_target_probability: 0.0` | Disables all curtain drops and curtain retreat. |
| Curtain for neighbour ships only | `ai_flares.launch_flares_probability: 0.0` | The targeted ship still rolls `launch_flares_target_probability`. |
| Curtain for the target ship only | `ai_flares.launch_flares_target_probability: 0.0` | Neighbouring ships can still start curtains using `launch_flares_probability`. |
| Curtain retreat steering | `ai_flares.launch_flares_evade: 0.0` | Curtain flare drops remain. |
| Curtain engine boost | `ai_flares.launch_flares_boost: 1.0` | Curtain drops and retreat remain. |
| Barrel rolls | `ai_flares.barrel_roll_probability: 0.0` | Plain and boosted panic evasion remain. |
| Boosted/full-steering panic evasion | `ai_flares.panic_evasion_probability: 0.0` | Plain evasion and barrel rolls remain. |
| Panic/roll engine boost | `ai_flares.panic_boost: 1.0` | Steering and roll angles remain. |
| Both dive directions | `tactics.dive_enabled: false` | Aim awareness and early fire remain. |
| Dive-under only | `tactics.dive_under: false` | Dive-over remains. |
| Dive-over only | `tactics.dive_over: false` | Dive-under remains. |
| Dive engine boost | `tactics.dive_boost: 1.0` | Dive steering remains. Works independently of `ai_flares.enabled`. |
| Aim awareness (dodging hostile gun aim) | `tactics.aim_enabled: false` | Early fire remains independent and can still roll on its window clock unless `aim_fire_probability: 0.0`. |
| Aim engine boost | `tactics.aim_boost: 1.0` | Aim steering remains. Works independently of `ai_flares.enabled`. |
| Early fire | `tactics.aim_fire_probability: 0.0` | Aim-awareness steering remains. |
| Tactical carrier plane groups | `plane_group_per_launch: 1` | Restores stock one-at-a-time cadence; strategic and manual launch paths are unchanged. |
| Zenith/missile dumping | `ai_missile_dump_chance: 0.0` | Restores vanilla launch timing for enemies and allies. Delay values do not disable dumping. |
| Panic/curtain trace | `logging.log_panic: false` | General startup and diagnostic messages in `Modloader.log` are separate. |

## Logging

All logging options live in `"logging"`. `log_panic` defaults to false and writes
`Modloader/logs/hf_ai_panic_<unix>.log`: text panic/curtain events with game time
and ship pointer. The old `ai_flares.log_panic` location remains readable; an
explicit `logging.log_panic` wins. Ship/steering/obstacle CSV capture has been
removed; old `log_steering`, `log_hz`, `log_scan` and `log_scan_bodies` keys do nothing.

## Top level

| Key | Default | If key omitted | What it does |
|---|---|---|---|
| `enabled` | `true` | Uses player default `true`. | Master switch. |
| `twr_cutoff` | `2.0` | Uses player default `2.0`. | Minimum live strategic/Shipworks TWR (`creature +0x160c`, before UI rounding) for tactics, missile evasion, rolls/boosts, curtain retreat, and stray-missile reactions. Below the cutoff these are suppressed; at or above it their individual switches still apply. Locked-missile flare bursts and preventive flare drops remain available. Negative/non-finite cutoffs use `0.0`. Independent of carrier plane groups. Restart after editing. |
| `plane_group_per_launch` | `3` | Uses player default `3`. | Clamped to `1..64`; `1` restores stock cadence. Enemy and allied AI, normal `Combat.seria` only. Stock deck/availability checks remain; blocked attempts cancel a partial group. Independent of TWR, tactics and flares. |
| `plane_group_launch_delay_seconds` | `0.5` | Uses player default `0.5`. | Clamped to `0.05..60` seconds; non-finite values use `0.5`. Stock six-second cooldown returns after the last member. Restart to apply. |
| `ai_missile_dump_chance` | `0.4` | Uses player default `0.4`. | Chance (`0..1`) that an AI ship (enemy, or ally when `ai_flares.allies` or `tactics.allies` is true) keeps launching Zenith missiles right after a launch instead of waiting for the game's 2 s cooldown. `0` = vanilla. |
| `ai_missile_dump_min_delay` | `0.6` | Uses player default `0.6`. | Shortest wait between the launches of a burst, in seconds. |
| `ai_missile_dump_max_delay` | `1.5` | Uses player default `1.5`. | Longest wait between the launches of a burst, in seconds (`max` never below `min`). |

## `ai_flares` (flare curtains, evasion, panic)

| Key | Default | If key omitted | What it does |
|---|---|---|---|
| `enabled` | `true` | Uses player default `true`. | Turns AI missile defence on. Enable it only when qol's `ai_flares.enabled` is `false`: both patch the same sites. |
| `allies` | `true` | Uses player default `true`. | Lets AI-run ships of the player's own side (allies/wingmen) use flares, missile evasion and curtains too. The ship you fly yourself is never affected. |
| `trigger_range` | `400.0` | Uses player default `400.0`. | How close, in game units, an incoming locked missile has to be before the ship reacts (`0..5000`). |
| `reaction_probability` | `0.6` | Uses player default `0.6`. | Chance, checked about ten times a second while a missile is inside the range, that the ship starts a panic (`0.0..1.0`). |
| `burst_seconds` | `1.5` | Uses player default `1.5`. | How long, in seconds, the launchers keep dropping flares once the panic starts (`0.05..60`). |
| `min_interval_seconds` | `1.5` | Uses player default `1.5`. | Quiet time, in seconds, after a panic ends before the same ship may panic again (`0..60`). |
| `evade` | `true` | Uses player default `true`. | Makes the ship also manoeuvre while it panics or runs a curtain. With `false` it only drops flares. Ships without flare launchers still evade when `evade` is `true`. |
| `evasion_seconds` | `3.0` | Uses player default `3.0`. | How long, in seconds, the evasive manoeuvre lasts, extended by 1 s while a locked missile stays in range (`0.05..60`). |
| `evasion_strength` | `0.5` | Uses player default `0.5`. | How much of the ship's course plain evasion takes over (`0.0..1.0`). |
| `panic_evasion_probability` | `0.1` | Uses player default `0.1`. | Chance, rolled once when a panic starts, that the evasion is a full-takeover boosted *panic evasion* (`0.0..1.0`). |
| `panic_evasion_seconds` | `5.0` | Uses player default `5.0`. | Duration of a panic evasion in seconds, extended by 2.5 s at a time (`0.05..60`). |
| `panic_boost` | `2.0` | Uses player default `2.0`. | Engine strength multiplier during a panic evasion or barrel roll (`1.0..4.0`). |
| `barrel_roll_probability` | `0.001` | Uses player default `0.001`. | Chance, rolled once when a panic starts, that the ship does a panic spin instead of evading (`0.0..1.0`). |
| `barrel_roll_seconds` | `2.0` | Uses player default `2.0`. | Duration of a barrel roll in seconds (`0.05..60`). |
| `stray_missiles` | `true` | Uses player default `true`. | Reacts (evasion only, no flare burst) when an unguided/other-target missile's trajectory passes close to the ship. |
| `launch_flares_probability` | `0.6` | Uses player default `0.6`. | Preventive flare curtain chance for ships near the missile's target (or all ships when `launch_flares_targeted_only` is `false`) (`0.0..1.0`). |
| `launch_flares_target_probability` | `1.0` | Uses player default `1.0`. | Preventive flare curtain chance for the specific ship the launched missile is aimed at (`0.0..1.0`). Always applies to the target ship regardless of `launch_flares_targeted_only`. |
| `launch_flares_targeted_only` | `true` | Uses player default `true`. | When `true`, only the targeted ship and ships within `launch_flares_neighbor_range` (`600u`) roll for a curtain. |
| `launch_flares_max_drops` | `6` | Uses player default `6`. | Maximum flare-release salvos per curtain (reset if a new launch extends the curtain). |
| `launch_flares_min_rack` | `0.5` | Uses player default `0.5`. | Minimum fraction (`0.0..1.0`) of a ship's peak flare capacity that must remain in its racks to drop curtain flares; checked before every drop so reserve flares are kept for a locked-on panic. Below this reserve (or with no flare launchers), the ship still performs the curtain retreat. |
| `launch_flares_evade` | `0.9` | Uses player default `0.9`. | How strongly (`0.0..1.0`) an active curtain steers the ship away from the incoming missile once the pre-drop ETA wait has elapsed. |
| `launch_flares_evade_range` | `400.0` | Uses player default `400.0`. | Maximum distance (game units) at which an active curtain steers away from a launched missile. |
| `launch_flares_boost` | `2.0` | Uses player default `2.0`. | Engine boost multiplier while curtain retreat steering is active (`1.0..4.0`). |

## `tactics` (universal aim dodging, early fire, and dive avoidance)

| Key | Default | If key omitted | What it does |
|---|---|---|---|
| `enabled` | `true` | Uses player default `true`. | Master switch for the `tactics` block. |
| `allies` | `true` | Uses player default `true`. | Also applies dive avoidance, aim dodging and early fire to AI-flown allied ships. The player-flown ship is never touched. |
| `dive_enabled` | `true` | Uses player default `true`. | "Deny the dive": sidestep when the target hangs inside the vertical cone above or below. |
| `dive_under` | `true` | Uses player default `true`. | React to a target hanging below the ship. |
| `dive_over` | `true` | Uses player default `true`. | React to a target hanging above the ship. |
| `dive_chance` | `0.8` | Uses player default `0.8`. | Chance (`0.0..1.0`) that a dive sidestep starts after `dive_dwell_s` in the cone (applies to both `dive_under` and `dive_over`). On a failed roll, the ship waits `dive_cooldown_s` and re-evaluates. |
| `dive_range` | `600.0` | Uses player default `600.0`. | Maximum distance (units) at which a target above/below triggers dive avoidance (`100..3000`). |
| `dive_dwell_s` | `0.5` | Uses player default `0.5`. | Seconds the target must stay in the vertical cone before rolling `dive_chance` (`0..30`). |
| `dive_cooldown_s` | `2.0` | Uses player default `2.0`. | Pause in seconds after a dive sidestep (or declined roll) before the next roll (`0..120`). |
| `dive_boost` | `2.0` | Uses player default `2.0`. | Engine boost multiplier while dive-dodging (`1.0..4.0`). Independent of `ai_flares.enabled`. |
| `aim_enabled` | `true` | Uses player default `true`. | Universal aim awareness: during each ship's staggered awareness window, it checks all hostile shooters (player and AI) and slips off the closest active aim ray crossing it. |
| `aim_chance` | `0.75` | Uses player default `0.75`. | Chance (`0.0..1.0`), rolled once per awareness window when a hostile aim ray crosses the ship, that it dodges for that window. Prevents endless synchronized juking in AI-vs-AI fights. |
| `aim_range` | `1800.0` | Uses player default `1800.0`. | Maximum distance (units) along a hostile shooter's aim ray at which a ship reacts (`100..5000`). |
| `aim_tolerance` | `200.0` | Uses player default `200.0`. | Maximum perpendicular miss distance (units) from a hostile aim ray that counts as aiming at the ship (`10..2000`). |
| `aim_boost` | `2.0` | Uses player default `2.0`. | Engine boost multiplier while dodging an aim ray (`1.0..4.0`). Independent of `ai_flares.enabled`. |
| `aim_require_fire` | `true` | Uses player default `true`. | Only dodge hostile aim rays from shooters that are actively firing (for the player, includes a 0.6 s hold-over after each volley so single-click cannon shots register; for AI shooters, checks active gun rate timers and burst latches). |
| `aim_fire_probability` | `0.5` | Uses player default `0.5`. | Offensive early fire (`0.0` = off): chance rolled once per awareness window when the ship's own target is within `aim_fire_range` x gun reach, its guns are aligned within ~20° of the target, and no friendly ship is in the line of fire. |
| `aim_fire_range` | `1.6` | Uses player default `1.6`. | Maximum target distance as a multiple of the ship's gun reach (`0.5..4.0`) for early fire. |

---

## Retired settings

### Retired in v0.10.11 (trimmed from `hf_ai.json`)

The keys below were removed from the default `hf_ai.json` preset in v0.10.11 to keep the configuration concise. They fall into two groups:

#### 1. Retired feature keys (ignored if present in JSON)

| Former key | Former default | Status in v0.10.11 |
|---|---|---|
| `ai_flares.launch_flares_shelter` | `true` | Retired. Ships without flares no longer steer behind a flare-dropping ally; they still evade/retreat from the missile directly. |
| `ai_flares.launch_flares_shelter_range` | `1200.0` | Retired with `launch_flares_shelter`. |
| `tactics.dive_under_chance` | `1.0` | Merged into `tactics.dive_chance` (`0.8`). |
| `tactics.dive_under_dwell_s` | `0.5` | Merged into `tactics.dive_dwell_s` (`0.5`). |

#### 2. Internal tuning keys (omitted from `hf_ai.json`; still accepted by the parser if added manually)

| Key | Compiled default | What it controls if added to `hf_ai.json` |
|---|---|---|
| `ai_flares.launch_flares_interval_min` | `0.5` | Shortest wait (s) between two drops of a flare curtain (`0.05..60`). |
| `ai_flares.launch_flares_interval_max` | `1.0` | Longest wait (s) between two drops of a flare curtain (`0.05..60`). |
| `ai_flares.launch_flares_seconds` | `5.0` | Active duration (s) of one flare curtain after its initial ETA wait (`0.1..60`). |
| `ai_flares.launch_flares_cooldown` | `6.0` | Quiet time (s) after a curtain ends before the next one may start (`0..60`). |
| `ai_flares.launch_flares_neighbor_range` | `600.0` | Distance (units) around the targeted ship within which neighbours also roll `launch_flares_probability` (`0..5000`). |
| `ai_flares.launch_flares_delay_min` | `1.0` | Minimum wait (s) after launch before a curtain activates (`0..30`). |
| `ai_flares.launch_flares_delay_max` | `2.0` | Fallback max wait (s) when missile ETA is unknown (`0..30`). |
| `ai_flares.launch_flares_eta_lead` | `4.0` | Seconds before estimated missile arrival (`distance / missile_speed`) at which the curtain activates (`0..10`). |
| `ai_flares.launch_flares_eta_max_wait` | `8.0` | Maximum pre-activation wait (s) for far-away missile launches (`0..30`). |
| `ai_flares.launch_flares_missile_speed` | `230.0` | Assumed missile speed (u/s) for curtain ETA calculation (`50..1000`). |
| `ai_flares.stray_missile_radius` | `150.0` | Miss distance (units, plus half ship radius) for stray-missile collision checks (`10..1000`). |
| `ai_flares.stray_missile_eta_s` | `3.0` | Maximum time to closest approach (s) for stray-missile evasion (`0.2..20`). |
| `ai_flares.stray_missile_range` | `1200.0` | Maximum distance (units) for stray-missile evasion (`100..10000`). |
| `tactics.ground_y` | `200.0` | Ground reference y (y grows downward) (`-5000..5000`). |
| `tactics.ground_clearance` | `450.0` | Clearance above `ground_y` below which tactics do not add downward steering (`0..5000`). |
| `tactics.ground_hard_y` | `120.0` | Hard floor y below which tactics never steer downward (`-5000..5000`). |
| `tactics.edge_margin` | `300.0` | Margin (units) from arena edges within which tactics do not add outward steering (`0..3000`). |
| `tactics.dive_cone_deg` | `45.0` | Half-angle (degrees) around straight up/down for dive detection (`5..85`). |
| `tactics.dive_min_s` | `1.5` | Shortest dive sidestep duration (s) (`0.2..30`). |
| `tactics.dive_max_s` | `3.0` | Longest dive sidestep duration (s) (`dive_min_s..60`). |
| `tactics.dive_weight` | `8.0` | Steering blend weight of the dive sidestep relative to the unit stock direction (`0..20`). |
| `tactics.dive_wrongfoot_speed` | `40.0` | Target horizontal speed (u/s) above which the sidestep goes against the target's motion (`0..1000`). |
| `tactics.dive_level_weight` | `0.7` | Vertical pull toward the target's altitude mixed into the dive sidestep (`0..3`). |
| `tactics.aim_window_s` | `1.5` | Length (s) of each per-ship awareness window (`0.1..30`). |
| `tactics.aim_gap_min_s` | `2.5` | Shortest start-to-start interval (s) between awareness windows (`aim_window_s..120`). |
| `tactics.aim_gap_max_s` | `3.5` | Longest start-to-start interval (s) between awareness windows (`aim_gap_min_s..300`). |
| `tactics.aim_weight` | `12.0` | Steering blend weight of the aim dodge relative to the unit stock direction (`0..20`). |
| `tactics.aim_fire_s` | `1.0` | Duration (s) of an offensive early-fire burst (`0.1..10`). |

### Retired in earlier versions (v0.10.4 – v0.10.6)

| Former key | Former default | Current effect | Replacement |
|---|---|---|---|
| `tactics.min_accel` | `35.0` | None (retired in v0.10.4). | Top-level `twr_cutoff` |
| `tactics.dive_min_accel` | `30.0` | None (retired in v0.10.4). | Top-level `twr_cutoff` |
| `tactics.apply_to_allies` | `false` | Accepted as a compatibility alias for `tactics.allies` (renamed in v0.10.5). | `tactics.allies` |
| `tactics.k_center`, `band_min`, `band_max`, `half_width`, `outrange_extra`, `blend_weight`, `strafe_min_s`, `strafe_max_s`, `retreat_tangent`, `strafe_enabled` | various | None; optional positioning routine removed in v0.10.6. | Stock AI positioning |
