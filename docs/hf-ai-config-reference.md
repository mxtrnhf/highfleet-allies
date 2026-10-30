# hf_ai settings (`Modloader/config/hf_ai.json`)

All keys are in the release pack's `hf_ai.json`. The plain-language explanation of how the AI works and what to turn for which effect is in [hf-ai-tuning-guide.md](hf-ai-tuning-guide.md); this file is the complete key list with the code defaults. Edit while the game is closed; `"enabled": false` turns hf_ai off.

## Missing keys and how to revert to vanilla

**You do not have to list every key.** Every top-level key, nested block and key inside a block may be omitted; hf_ai supplies the code default shown below. For example, `{ "tactics": { "enabled": true } }` enables tactics with default tuning and leaves everything else at its default. An empty `{}` is also valid. If the file is absent, hf_ai tries to create it with the full defaults; if the JSON is invalid, it logs an error and uses the defaults.

**Default does not always mean vanilla.** The release pack enables features that are off in the code defaults, and some defaults only tune an already-enabled feature. Use the explicit values below when you want stock behaviour. There is no general `-1` disable convention.

For the most complete revert, use only:

```json
{ "enabled": false }
```

To keep hf_ai loaded but restore stock gameplay and disable its optional data files, use:

```json
{
  "enabled": true,
  "log_steering": false,
  "log_scan": false,
  "log_scan_bodies": false,
  "ai_missile_dump_chance": 0.0,
  "ai_flares": { "enabled": false, "log_panic": false },
  "tactics": { "enabled": false }
}
```

This leaves hf_ai's pass-through steering hook installed; `"enabled": false` avoids that too. QoL has independent flare and missile-dump settings, so revert those separately if they are enabled.

### Feature-by-feature vanilla settings

| Feature | Setting that disables or neutralizes it | What remains / important dependency |
|---|---|---|
| Entire mod | `"enabled": false` | No hf_ai gameplay hooks are installed. |
| All flare, panic and curtain behaviour | `ai_flares.enabled: false` | Tactics and missile dump remain; tactics engine boosts lose the shared hook they need. |
| All threat-triggered panics, locked and stray | `ai_flares.reaction_probability: 0.0` | No new threat panic, flare burst, roll or panic evasion. Preventive curtains are separate and remain if enabled. |
| Locked-missile flare bursts only, while keeping locked evasion | **No independent off switch** | `burst_seconds: 0.0` is clamped to `0.05`, not off. Use `reaction_probability: 0.0` to stop all threat panics, or disable the whole flare subsystem. |
| All flare-related manoeuvres | `ai_flares.evade: false` | Locked-missile flare bursts and curtain drops can still run; plain/panic evasion, rolls and curtain retreat stop. |
| Plain evasion steering only | `ai_flares.evasion_strength: 0.0` | Neutralizes its steering contribution, but the panic state, boosted panic evasion and rolls remain possible. |
| Stray/lost-lock missile evasion | `ai_flares.stray_missiles: false` | Locked-missile behaviour remains. |
| Allied flares, panic, curtains and shared engine boosts | `ai_flares.allies: false` | Enemy behaviour remains. The player-flown ship is always excluded. |
| Allied tactical steering | `tactics.apply_to_allies: false` | Independent of `ai_flares.allies`; set both to false to exclude allies from everything. |
| Preventive flare curtains, including target ships | `ai_flares.launch_flares_probability: 0.0` | Disables the whole curtain even when `launch_flares_target_probability` is nonzero: drops, retreat and shelter. |
| Curtain for the target ship only | `ai_flares.launch_flares_target_probability: 0.0` | Neighbouring ships can still start curtains using `launch_flares_probability`. |
| Curtain drops only, while preserving retreat | **No independent drop-free-retreat switch** | `launch_flares_max_drops: 0` ends drop-capable curtains instead. Disable the whole curtain with `launch_flares_probability: 0.0`. |
| Curtain retreat steering | `ai_flares.launch_flares_evade: 0.0` | Curtain drops remain. |
| Curtain sheltering behind another ship | `ai_flares.launch_flares_shelter: false` | Ships use ordinary away-from-missile retreat instead. |
| Curtain engine boost | `ai_flares.launch_flares_boost: 1.0` | Curtain drops and retreat remain. |
| Barrel rolls | `ai_flares.barrel_roll_probability: 0.0` | Plain and boosted panic evasion remain. |
| Boosted/full-steering panic evasion | `ai_flares.panic_evasion_probability: 0.0` | Plain evasion and barrel rolls remain. |
| Panic/roll engine boost | `ai_flares.panic_boost: 1.0` | Steering and roll angles remain. For guaranteed no panic engine rewrites, also set both panic-evasion and barrel-roll probabilities to `0.0`. |
| Strafing/band positioning and close retreat | `tactics.strafe_enabled: false` | Dive, aim awareness and early fire remain. Use this switch rather than `blend_weight: 0.0` for a true off. |
| Both dive directions | `tactics.dive_enabled: false` | Strafe, aim awareness and early fire remain. |
| Dive-under only | `tactics.dive_under: false` | Dive-over remains. |
| Dive-over only | `tactics.dive_over: false` | Dive-under remains. |
| Dive wrong-foot direction choice | `tactics.dive_wrongfoot_speed: 0.0` | Direction becomes random except near arena walls; diving remains. |
| Dive vertical level pull | `tactics.dive_level_weight: 0.0` | Dive becomes purely sideways; it is not disabled. |
| Dive engine boost | `tactics.dive_boost: 1.0` | Dive steering remains. Boosts require `ai_flares.enabled: true`. |
| Aim awareness and its steering | `tactics.aim_enabled: false` | Also prevents early fire, which is rolled from the aim state. |
| Aim engine boost | `tactics.aim_boost: 1.0` | Aim steering and early fire remain. Boosts require `ai_flares.enabled: true`. |
| Early fire | `tactics.aim_fire_probability: 0.0` | Aim-awareness steering remains. |
| Zenith/missile dumping | `ai_missile_dump_chance: 0.0` | Restores vanilla launch timing. Delay values do not disable dumping. |
| Steering CSV | `log_steering: false` | `log_hz: 0.0` does not disable it; the rate is clamped. |
| Scan/obstacle CSVs | `log_scan: false` | Also disables `log_scan_bodies` output. |
| Considered-body rows in scan logs | `log_scan_bodies: false` | Other scan-log rows remain while `log_scan` is on. |
| Panic/curtain trace | `ai_flares.log_panic: false` | General startup and diagnostic messages in `Modloader.log` are separate. |

## Top level
| Key | Default | What it does |
|---|---|---|
| `enabled` | `true` | Master switch. |
| `log_steering` | `true` | Write `Modloader/logs/hf_ai_steer_<unix>.csv`: the stock AI's wanted waypoint per ship. |
| `log_hz` | `10.0` | Rows per second per ship in that file. |
| `log_scan` | `false` | Write `hf_ai_scan_<id>.csv` / `hf_ai_obst_<id>.csv`: every AI scan call with the obstacle nodes and the steering numbers the game computed (50 Hz per ship). Off by default; see `scan_log.rs`. |
| `log_scan_bodies` | `false` | With `log_scan`: also list every body the scan considered (only on ticks with a kept node). |
| `ai_missile_dump_chance` | `0.0` | Chance (0..1) that an enemy ship keeps launching Zenith missiles right after a launch (a burst that empties the rack) instead of waiting for the game's 2 s cooldown. 0 = vanilla. Was `ai_missile_dump_chance` in qol.json. |
| `ai_missile_dump_min_delay` | `0.1` | Shortest and longest wait between the launches of a burst, in seconds (`max` never below `min`). |
| `ai_missile_dump_max_delay` | `0.5` | Longest wait between launches of a burst, seconds. |

## `ai_flares` (flare curtains, evasion, panic)
| Key | Default | What it does |
|---|---|---|
| `enabled` | `false` | Turns AI flares on. Disabled by default. Enable it only when qol's `ai_flares.enabled` is `false`: both patch the same three sites and whichever loads second finds other bytes and skips. |
| `trigger_range` | `400.0` | How close, in game units, an incoming missile has to be before the ship reacts.  Limited to `0` through `5000`. |
| `reaction_probability` | `0.6` | Chance, checked about ten times a second while a missile is inside the range, that the ship starts a burst. `1.0` reacts at once, `0.1` makes the ship slow and unreliable.  Limited to `0.0` through `1.0`. |
| `burst_seconds` | `1.5` | How long, in seconds, the launchers keep dropping flares once the panic starts. Each launcher drops one flare per its own firing interval (0.15 s for the vanilla launcher), so 1.5 s is about ten flares per launcher.  Limited to `0.05` through `60`. |
| `min_interval_seconds` | `1.5` | Quiet time, in seconds, after a panic ends before the same ship may panic again.  Limited to `0` through `60`. |
| `evade` | `true` | Makes the ship also manoeuvre while it panics. With `false` it only drops flares. |
| `allies` | `true` | Lets AI-run ships of the player's own side (allies brought in by a mod, wingmen) use flares and evasion too. The game's own check is "negative side = enemy", so friendly AI ships never did. The ship you fly yourself is never affected. Enabled by default. |
| `evasion_seconds` | `3.0` | How long, in seconds, the evasive manoeuvre lasts. It is extended by one second each time a locked-on missile is still inside the range.  Limited to `0.05` through `60`. |
| `evasion_strength` | `0.9` | How much of the ship's course the evasion takes over: `0.0` none, `1.0` all of it.  Limited to `0.0` through `1.0`. |
| `barrel_roll_probability` | `0.01` | Chance, rolled once when a panic starts, that the captain panics so hard the ship does a panic spin (what the player's Q or E with Shift does: the engines on one side push up and the others down, boosted by `panic_boost`; flares still drop) instead of evading.  Limited to `0.0` through `1.0`. |
| `barrel_roll_seconds` | `2.0` | How long, in seconds, the panic spin lasts. It is not extended.  Limited to `0.05` through `60`. |
| `panic_evasion_probability` | `0.3` | Chance, rolled once when a panic starts, that the evasion is a *panic evasion*: the ship ignores its normal steering completely, picks one escape direction and floors it that way and boosts its engines like a player holding Shift. The other panics use the plain evasion above.  Limited to `0.0` through `1.0`. |
| `panic_boost` | `2.0` | Engine strength during a panic evasion: `1.0` is normal thrust, `2.0` is what the player's boost key gives.  Limited to `1.0` through `4.0`. |
| `panic_evasion_seconds` | `8.0` | How long, in seconds, a panic evasion lasts. A missile that is still coming extends it by 2.5 s at a time.  Limited to `0.05` through `60`. |
| `log_panic` | `false` | Writes `Modloader/logs/hf_ai_panic_<unix>.log`: one line per panic event (start, extension, end, ignored reaction roll, first panic-evasion direction, spin/boost) with the game time and the ship pointer. Disabled by default. |
| `launch_flares_probability` | `0.0` | "Flare curtain": chance, rolled once per enemy ship each time the player's side launches a missile, that the ship starts dropping flares at random intervals BEFORE any lock-on (like a helicopter flooding the area with fake heat sources). `0.0` = off. Enemy ships only.  Limited to `0.0` through `1.0`. |
| `launch_flares_interval_min` | `0.5` | Shortest and longest wait between two drops of a curtain, in seconds (every drop fires all launchers). |
| `launch_flares_interval_max` | `1.0` | Longest wait between two flare drops of a curtain, seconds (min is `launch_flares_interval_min`). |
| `launch_flares_seconds` | `5.0` | How long one curtain lasts; another launch while it runs extends it. |
| `launch_flares_max_drops` | `6` | Most drops of one curtain, so the racks are not empty when the real panic needs them. |
| `launch_flares_cooldown` | `6.0` | Quiet time after a curtain before the next one may start. |
| `launch_flares_evade` | `0.9` | While a curtain runs the ship moves away from the launched missile: `0.0` none, `1.0` all of its course. |
| `launch_flares_evade_range` | `3000.0` | Only missiles closer than this (game units) count for that retreat. |
| `launch_flares_boost` | `2.0` | Engine boost while that retreat runs (`1.0` = none, `2.0` = like the player's Shift).  Limited to `1.0` through `4.0`. |
| `launch_flares_min_rack` | `0.5` | A ship only drops flares in a curtain while its racks hold at least this share of the most flares it was seen to carry (so some are left for a real panic). Below it, or with no flare launcher at all, the curtain only retreats. `0.0` = always drop.  Limited to `0.0` through `1.0`. |
| `launch_flares_targeted_only` | `true` | Preventive defence only: a curtain starts only for the ship the launched missile is aimed at, or a ship within `launch_flares_neighbor_range` of it. `false` = every enemy ship rolls (the v0.5.0 behaviour). |
| `launch_flares_neighbor_range` | `600.0` | How close (game units) a ship must be to the targeted ship to join its curtain. `0` = only the targeted ship.  Limited to `0` through `5000`. |
| `launch_flares_shelter` | `true` | A curtain ship that has no flares at all (no launcher) heads for an ally that is dropping flares (so the flares sit between the missile and it) instead of just running away from the missile. |
| `launch_flares_shelter_range` | `1200.0` | Only allies closer than this (game units) count as shelter.  Limited to `0` through `5000`. |
| `launch_flares_delay_min` | `1.0` | A curtain waits a random time in this range (seconds) after the launch before its first drop, so the flares are still in the air when the missile arrives (they disappear after a short time).  Limited to `0` through `30`. |
| `launch_flares_delay_max` | `2.0` | Longest random delay before a curtain starts, seconds (min is `launch_flares_delay_min`). |
| `launch_flares_eta_lead` | `4.0` | With a known missile ETA (distance / `launch_flares_missile_speed`) the first drop is `ETA - launch_flares_eta_lead` seconds after the launch, never later than `launch_flares_eta_max_wait`. Flares live 5 s, so a lead near 4 puts the cloud in the air when the missile arrives. |
| `launch_flares_eta_max_wait` | `8.0` | Longest the curtain waits for the missile's estimated arrival before dropping, seconds. |
| `launch_flares_missile_speed` | `230.0` | Assumed missile speed (units/s) for the arrival estimate: distance / this = ETA. |
| `launch_flares_target_probability` | `1.0` | Chance for the ship the missile is aimed at (the neighbours use `launch_flares_probability`). |
| `stray_missiles` | `true` | Stray missiles: a missile that is NOT locked on to the ship (it lost its lock or is aimed at someone else) but flies a path that passes within `stray_missile_radius` (+ half the ship's size) of the ship within `stray_missile_eta_s` seconds, closer than `stray_missile_range`, triggers evasion only, without a flare burst. A locked missile can still add one flare burst during that evasion. |
| `stray_missile_radius` | `150.0` | A loose missile (not locked on) counts as a threat if its straight path passes within this distance of the ship (units). Raise for a more nervous AI. |
| `stray_missile_eta_s` | `3.0` | ...and it would arrive within this many seconds. |
| `stray_missile_range` | `1200.0` | ...and it is closer than this distance (units). |

## `tactics` (dodging, aim awareness, strafing)
| Key | Default | What it does |
|---|---|---|
| `enabled` | `false` | Master switch (A/B tests). |
| `apply_to_allies` | `false` | Also steer AI ships of the player's side (allies). The ship you fly is never touched. |
| `min_accel` | `35.0` | Ships with `(left + right engine thrust) / mass` below this stay stock (Gryphon 22 .. Lightning 128). |
| `k_center` | `0.32` | Band centre = `k_center` x own gun reach, limited to `band_min..band_max`. |
| `band_min` | `300.0` | Lowest centre of the strafing distance band (units). |
| `band_max` | `550.0` | Highest centre of the strafing distance band (units). |
| `half_width` | `100.0` | Half the band width: strafe between centre - half_width and centre + half_width. |
| `outrange_extra` | `150.0` | Extra distance when the target's gun reach is longer than ours. |
| `blend_weight` | `1.0` | Weight of the tactic direction against the unit stock direction (0 = stock). |
| `strafe_min_s` | `2.0` | The strafe side reverses after a random time in this range. |
| `strafe_max_s` | `5.0` | Longest time before the strafing direction flips (min is `strafe_min_s`). |
| `retreat_tangent` | `0.7` | Tangential share in the too-close retreat (0 = straight back). |
| `ground_y` | `200.0` | Ground reference y (y grows downward) and the clearance above it where no downward component is added. |
| `ground_clearance` | `450.0` | Height above the ground (units) that strafing/dodging keeps. |
| `ground_hard_y` | `120.0` | Below this y no tactic steers downward at all (the ground is about +250; y grows downward). |
| `edge_margin` | `300.0` | No outward component within this distance of an arena edge. |
| `strafe_enabled` | `true` | "Strafe in the band" on/off (`enabled` stays the master switch). |
| `dive_enabled` | `true` | "Deny the dive": sidestep when the target hangs straight below / above. Own switch, works with the strafe off. |
| `dive_chance` | `0.8` | Chance (0..1) that the sidestep starts; rolled once each time the target enters the cone. |
| `dive_cone_deg` | `45.0` | Half-angle in degrees around straight down / up. |
| `dive_range` | `600.0` | Targets above/below count only within this distance (units). |
| `dive_dwell_s` | `1.0` | The target must stay in the cone this long before the roll. |
| `dive_min_s` | `1.5` | Shortest sidestep after the dive trigger (seconds). |
| `dive_max_s` | `3.0` | Longest sidestep (seconds). |
| `dive_cooldown_s` | `2.0` | Pause after a sidestep before the next one (seconds). |
| `dive_weight` | `8.0` | How strongly the sidestep replaces the stock waypoint. |
| `dive_min_accel` | `30.0` | Ships below this thrust/mass never sidestep. |
| `dive_under` | `true` | React to a target below us and / or above us. |
| `dive_over` | `true` | Dodge a target hanging ABOVE the ship (on/off). |
| `dive_boost` | `2.5` | Engine boost while dodging (1.0 = off, 2.0 = like Shift). Needs `ai_flares.enabled` (its engine hook carries it). |
| `dive_wrongfoot_speed` | `40.0` | Wrong-foot speed (u/s): a target moving sideways faster than this makes the dodge go the other way (0 = random side). |
| `dive_under_chance` | `1.0` | Own chance and dwell for a target BELOW us (the damaging case); `dive_chance` / `dive_dwell_s` apply to a target above. |
| `dive_under_dwell_s` | `0.5` | How long a target must hang below before the ship dodges (seconds). |
| `dive_level_weight` | `0.7` | Vertical pull onto the target's altitude mixed into the dodge (0 = purely sideways). |
| `aim_enabled` | `false` | "Aim awareness" (off by default): in shared awareness windows the enemies that target the player see the player's aim line and slip off it. Window length, random time from the start of one window to the start of the next, hit tolerance (miss distance from the aim ray), range along the ray, sidestep weight and engine boost. |
| `aim_window_s` | `1.5` | How long each awareness window stays open (seconds). |
| `aim_gap_min_s` | `2.5` | Shortest pause between awareness windows (seconds). |
| `aim_gap_max_s` | `3.5` | Longest pause between windows (seconds). |
| `aim_tolerance` | `200.0` | A ship within this distance (units) of your aim line slips off it. |
| `aim_range` | `1800.0` | Only ships closer than this (units) react. |
| `aim_weight` | `12.0` | How strongly the ship leaves the aim line. |
| `aim_boost` | `2.5` | Engine boost factor while dodging the aim line (needs `ai_flares.enabled`). |
| `aim_require_fire` | `true` | The aim dodge only happens while the player is actually shooting (either gun group's fire byte set). |
| `aim_fire_probability` | `0.0` | Early fire (0 = off): chance, rolled once per awareness window for a ship on the aim line, that it opens fire with its main guns for `aim_fire_s` seconds while the player is within `aim_fire_range` x its gun reach. |
| `aim_fire_s` | `1.0` | How long an early-fire burst lasts (seconds). |
| `aim_fire_range` | `1.6` | Early fire only if you are within this multiple of the ship's gun reach. |
