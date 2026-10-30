# Tuning hf_ai: a plain-language guide (v0.9.0)

Written 2026-10-24 for the person who tunes the AI, not for the person who reads the code. Every number below is a key in `Modloader/config/hf_ai.json`; the code defaults are in `hf_ai/src/tactics_math.rs` (`Params::default`) and `hf_ai/src/flares_config.rs`. Where I only know something from reading the code and not from playing, it says so.

## 1. Is "state machine" the right word?

Yes, with two corrections.

* **Each AI ship has its own small state machine.** At any moment it is in exactly one state (`stock`, `far`, `band`, `close`, `dive`, `dive_skip`, `aim`, `panic`; the names are the `state` column of `hf_ai_steer_*.csv`). It moves between states on conditions (target in the dive cone for long enough, an aim window opens, a missile locks on) and on timers (dwell, cooldown, duration).
* **It is not the game's AI.** The game's own AI runs every tick and computes a *waypoint* (where to fly next). hf_ai sits on that one point: it reads the waypoint the game wanted and either leaves it (`stock`) or replaces it. It never touches guns, targets or missiles, except the optional early fire. So "the state machine" is a steering override on top of vanilla, not a replacement brain.
* **There are really two machines.** (a) The *steering* machine above (file `tactics_math.rs`, `tactics.rs`). (b) The *flare curtain* machine (`curtain_math.rs`, `ai_flares.rs`): wait, drop, cooldown, driven by missile launches. They share the ship and the panic, but have separate settings (`tactics` block vs `ai_flares` block).

## 2. What happens every tick

1. **Gates** (cheap checks first). hf_ai stays out (`stock`) unless: the ship is AI-flown (never the ship you fly), it is airborne (mode 1), it has a living target that is not itself, the arena bounds are readable, and the ship is on a side that is allowed (enemy always; your allies only with `allies`).
2. **TWR gate.** The top-level `twr_cutoff` is the sole capability threshold. `0.0` admits every ship; a positive value admits battle TWR at or above that value. Individual tactic switches and safety checks still apply. The former strafe/dive acceleration settings are deprecated and ignored.
3. **Priority order** (first match wins): **panic** > **aim** > **dive** > **strafe (far / band / close)** > stock.
4. The winning state produces a direction. It is mixed with the game's own direction as `stock_direction + weight * tactic_direction` and re-normalised, so a large weight means "mostly my direction", weight 0 means stock. Ground, arena edges and "never steer down when low" limits are applied after.
5. Some states ask for an engine boost (like holding Shift). **The boost is applied by the `ai_flares` engine hook, so `dive_boost` / `aim_boost` do nothing unless `ai_flares.enabled` is true.**

## 3. The states and what each knob does

### 3.1 Panic (the `ai_flares` block): incoming missile
Triggers when a missile that has locked on to the ship comes within `trigger_range` (400), or—with `stray_missiles`—when a loose one's straight-line path passes close. A locked missile starts flares and evasion; a stray starts evasion only, and a later lock can add one flare burst. Each check (about 10 per second) rolls `reaction_probability` (0.6): a failed roll means no reaction yet, so lower values make the AI slower and sloppier.
* It drops flares for `burst_seconds` (1.5 s, about 10 flares per launcher), then waits `min_interval_seconds` before the next panic.
* `evade` true: it also steers sideways (perpendicular to the missile's line of approach) for `evasion_seconds`, `evasion_strength` 0..1 = how much of its course is taken over. Extended by 1 s each time a locked missile is still inside the range.
* With `panic_evasion_probability` (rolled once per panic) it instead commits to one escape direction at `panic_boost` x thrust for `panic_evasion_seconds` (extended 2.5 s at a time). It prefers the higher sidestep and never dives toward the ground when low.
* `barrel_roll_probability`: the "panic spin" (engines on one side up, other side down). Spectacular, rarely useful; keep near 0 to 0.02.
* `stray_missile_*`: radius / ETA / range for loose missiles. Raise the radius for a more nervous AI.

*Tuning feel:* difficulty goes up with `reaction_probability`, down with `min_interval_seconds` and up with `trigger_range`. The measured fact behind the defaults: enemy missiles hit only 2 to 5 percent of the time, so do not make panic perfect or missiles become useless.

### 3.2 Flare curtain (preventive flares)
When the **other camp** launches a missile, an AI ship rolls once whether to start dropping flares at random intervals *before* any lock. For enemy ships the trigger is your missile; **for allies (v0.9.0) it is an enemy missile aimed at you or at an ally.** Allies need `ai_flares.allies` true.
* `launch_flares_probability` (release: 0.6) for ships near the target; `launch_flares_target_probability` (1.0) for the ship the missile is aimed at. `launch_flares_targeted_only` true: only the targeted ship and ships within `launch_flares_neighbor_range` (600) join; false: every ship rolls.
* **Timing.** Flares live exactly 5 s. The first drop is timed from the missile's ETA (distance / `launch_flares_missile_speed` 230) minus `launch_flares_eta_lead` (4 s), clamped to `launch_flares_delay_min` (1) .. `launch_flares_eta_max_wait` (8). If the cloud is dropped too early it is gone when the missile arrives (this was the main v0.7.0 defect: 48 percent of drops were wasted). Keep the lead near 4.
* **Budget.** `launch_flares_max_drops` (6) per curtain, `launch_flares_interval_min/max` (0.5..1.0) between drops, `launch_flares_min_rack` (0.5): the ship stops dropping when the racks are below that share, so some flares stay for a real panic. `launch_flares_cooldown` (6) between curtains.
* **Movement.** While the curtain runs the ship moves away from the missile (`launch_flares_evade` 0.9, `launch_flares_boost` 2.0, within `launch_flares_evade_range` 3000). A ship with no flare launcher shelters 250 u *behind* a dropping friend (`launch_flares_shelter`, range 1200).

*Tuning feel:* to make missiles more dangerous, lower the probabilities or raise `launch_flares_eta_lead` away from 4 (mis-timed flares). To make the AI feel smart, raise `launch_flares_target_probability` and keep `shelter` on.

### 3.3 Dive ("deny the dive")
If the target hangs within `dive_cone_deg` (45) of straight below ("under") or above ("over") the ship, closer than `dive_range` (600), for the dwell time, the ship rolls once per time the target enters the cone. On success it sidesteps horizontally for `dive_min_s..dive_max_s` (1.5..3), then pauses `dive_cooldown_s` (2). On a failed roll it is `dive_skip` and stays stock until the target leaves the cone.
* Why it exists: a target straight below you is the one that can hit your keel. Sidestepping and pulling to its altitude (`dive_level_weight` 0.7, full strength from 150 u of height difference) takes that shot away.
* Under vs over have separate odds: `dive_under_chance` (1.0) and `dive_under_dwell_s` (0.5) for a target below; `dive_chance` (0.8) and `dive_dwell_s` (1.0) for one above. `dive_under` / `dive_over` switch either off.
* `dive_wrongfoot_speed` (40 u/s): if the target is moving sideways faster than this, the ship dodges the *other* way, so you must reverse your momentum to follow. 0 means a random side. A near arena wall always wins over the choice.
* Strength: `dive_weight` (8) and `dive_boost` (2.5, needs ai_flares on).

*Tuning feel:* "too passive / still gets shot from below": `dive_under_chance` is already 1.0, so lower `dive_under_dwell_s`, raise `dive_weight` / `dive_boost`, widen `dive_cone_deg`. "Looks like it is dancing": lengthen `dive_cooldown_s`, lower the chances.

### 3.4 Aim awareness (enemies only so far)
Every `aim_gap_min_s..aim_gap_max_s` (2.5..3.5 s) an *awareness window* of `aim_window_s` (1.5 s) opens for all ships at once. During it, enemies that target **you** look at the line your guns point along. A ship whose position is within `aim_tolerance` (200 u) of that ray, closer than `aim_range` (1800 u), slips off the line sideways (`aim_weight` 12, `aim_boost` 2.5).
* `aim_require_fire` true: only while you are actually pressing fire (either gun group). This was added because 24 percent of on-line time you were not shooting.
* **Early fire:** `aim_fire_probability` (release 0.5): once per window a ship that is on the line rolls this chance and opens fire with its main guns for `aim_fire_s` (1 s) if you are within `aim_fire_range` (1.6) x its gun reach. Bypasses the stock range, angle and friendly-fire tests, so allies near the line of fire can be hit. Set 0 to turn it off.
* **Allies do not get aim awareness in v0.9.0.** It needs each enemy's gun direction, which hf_ai does not read yet (only the player's aim is known, at game address `0x1403c0dc8`).

*Tuning feel:* "too hard to hit": raise `aim_gap_*` (longer pauses between windows) or lower `aim_weight`/`aim_boost`; narrow `aim_tolerance`. The effect is small by design: the measured displacement was only about 17 u/s before the weights were raised.

### 3.5 Strafe (far / band / close): OFF in the release
Keeps the ship in a distance band around the target (centre = `k_center` 0.32 x its gun reach, limited to `band_min..band_max` 300..550, width +-`half_width` 100, longer if the target outranges it by `outrange_extra`), alternating the sideways direction every `strafe_min_s..strafe_max_s`. `far` = beyond the band (stock charge), `band` = strafing, `close` = retreating with a tangent (`retreat_tangent`). It is off because the stock AI already orbits at about 0.35 x reach, so in the band it mostly cancels itself; the test showed band rows only about 12 percent and `close` retreat followed 70 percent of the time. If you try it, raise `blend_weight` to 2 first.

## 4. Ally differences (v0.9.0)

| Feature | Allies |
|---|---|
| Panic and flares, stray missiles | yes (`ai_flares.allies` true) |
| Flare curtain | yes, triggered by **enemy** launches at the player's side |
| Dive | yes (`tactics.allies` true), against the ally's own target |
| Strafe | only if strafe is on |
| Aim awareness / early fire | no |
| The ship you fly | never touched |

Unconfirmed in play: whether allies really dodge as intended. The ally curtain and dive are new and were checked only by unit tests (80 pass), not in a battle. Look for `CURTAIN_START` lines whose ship pointer is an ally in `hf_ai_panic_*.log`.

## 5. How to tune without guessing

1. **Change one thing at a time**, keep a `.bak` of `hf_ai.json`, and change `enabled` (master) or one block's `enabled` to A/B. A fight against the same enemy design (say Intrepid) is the comparison unit.
2. **Turn logging on for the test only:** `"log_steering": true` (steer CSV, 10 Hz, the `state` column shows where each ship was), `"log_scan": true` (obstacle scan, bigger), `ai_flares.log_panic` true (panic and curtain events with timestamps). They are only written while hf_ally's battle session is open.
3. **Look at the replay**, not only the CSV: `build_viz.py` builds an HTML replay from the telemetry (`tuning_telemetry/visualizer/` in the workspace).
4. **Measure, then judge.** Useful numbers: share of enemy missiles that hit; share of curtain drops still in the air when the missile arrives (target: most); how often each `state` appears; time under your keel before the ship moves (dive); damage taken per minute. Do not judge from one fight: dice are rolled per ship.
5. **Safe order for a "harder" AI:** raise `reaction_probability`, then the dive weights, then the aim window frequency, last the early fire. For an "easier" one reverse it. Early fire is the one that makes the AI visibly dangerous, so move it in steps of 0.1.
6. **Pitfalls.** (a) Boosts need `ai_flares.enabled` true. (b) Qol's own `ai_flares` must be false while hf_ai's is true: both patch the same sites. (c) A ship under 30 thrust/mass never does anything, so heavies feel "stock". (d) Settings are read at game start (no hot reload that I know of; restart the game after editing). (e) Wrong key names are silently ignored (every key is optional), so check the `hf_ai: tactics ON (...)` log line shows your numbers.

## 6. Release defaults (pack v0.13.0-rel.3)

`enabled` true; `ai_flares`: enabled, allies, evade, `panic_evasion_probability` 0.3, `launch_flares_probability` 0.6, `log_panic` true, everything else at the code defaults; `tactics`: enabled, `allies` true, `strafe_enabled` false, `dive_enabled` true, `aim_enabled` true, `aim_fire_probability` 0.5; steering and scan logs off.

## 7. Recipes: "I see X, so I change Y"

All keys live in `Modloader/config/hf_ai.json`. Keep a `.bak`, change one line, restart the game, fight the same enemy again. Unlisted keys keep the defaults in sections 3.1 to 3.4.

| What you see in battle | What to change |
|---|---|
| Too many flares around the targeted ship; neighbours dump for nothing | `ai_flares.launch_flares_neighbor_range` to `0`: only the ship the missile is aimed at joins the curtain. Lower `launch_flares_probability` (0.6) to thin it out further. |
| Enemies dump flares every time I (or my allies) fire anything | They react to any missile launched by your side. Lower `launch_flares_probability`, or set `launch_flares_targeted_only` true (it is the default) so only the target reacts. |
| Flares are gone before the missile arrives | `launch_flares_eta_lead` is the head start in seconds (4). Flares last 5 s, so keep it near 4. |
| My allies dump flares too often | `ai_flares.allies` false turns allies' flares off completely. Otherwise lower `launch_flares_probability` (it applies to everyone). |
| Ships with no flare launchers just sit there when a Zenith comes | Shelter and running away already exist (section 3.2). Make them braver or more nervous with `launch_flares_evade` (0.9), `launch_flares_boost` (2.0) and `launch_flares_evade_range` (3000). `launch_flares_shelter` must stay true. |
| The AI is too good at dodging missiles | Lower `reaction_probability` (0.6) and `panic_evasion_probability` (0.3); raise `min_interval_seconds`. |
| The AI is too easy to hit with missiles | The opposite: raise `reaction_probability`, `trigger_range` (400) and `stray_missile_*` radius. |
| Ships keep shooting at me from below | Section 3.3: lower `dive_under_dwell_s`, raise `dive_weight` / `dive_boost`. |
| Enemies hit me too early or from far away | `aim_fire_probability` to `0` turns off early fire. |
| Heavy ships never dodge | Check `twr_cutoff` and the individual tactic switches. Since v0.10.4 there is no separate acceleration gate; `0.0` removes the TWR restriction. |

## 8. Why flares start at "wrong" times (what the code does)

* **Missile locked on a ship** (panic, section 3.1): flares start only when the missile is closer than `trigger_range` (400 units) and the reaction roll passed. This is the "late" flare burst.
* **Missile launched by the other camp** (curtain, section 3.2): flares start before any lock, a few seconds before the missile's estimated arrival. This is the "early" one, and it is the reason a ship can dump flares when nothing is yet aimed at *it*: it is the target or stands within `launch_flares_neighbor_range` of the target. The trigger is the launch, not the danger to that particular ship.
* **Allies** run the same logic mirrored: they dump on an **enemy** launch aimed at you or at an ally.
* **K-13 (carrier planes' air-to-air missile):** checked in the 2026-10-25 test logs (6 K-13 launches with hf_ai loaded): no launch was detected and no curtain started, so the K-13 does not set off flare dumps. Not checked: enemy planes firing K-13 at your allies.
* **Shells:** nothing in hf_ai dodges shells. Only the dive and aim tactics (3.3, 3.4) affect where ships fly relative to your guns.
* **Zeniths and ships without flares:** they use the panic evasion and the curtain's shelter / run-away. How good that is has not been measured separately; enemy missiles hit only 2 to 5 percent overall.

## 9. A one-battle test routine

1. Set `"log_panic": true` inside `ai_flares`.
2. Fight, then open the newest `Modloader/logs/hf_ai_panic_*.log`. `CURTAIN_START` is a preventive dump (the line says which launch triggered it), `START` is a panic, `CURTAIN_DROP` is each flare drop.
3. Count: how many `CURTAIN_START` lines happened without you or an ally launching anything? If zero, nothing is "wrong", you are seeing the design. If not zero, send the log.
4. Change one value from section 7 and repeat.

## Missile dump (v0.10.0)
Enemy ships normally launch one Zenith, then wait 2 s. With `ai_missile_dump_chance` above 0 an enemy that just launched keeps launching with that chance, so a rack empties in a burst; `ai_missile_dump_min_delay` / `ai_missile_dump_max_delay` (seconds) are the random wait between launches (a delay of 0.6 to 1.5 stops missiles blowing up their neighbours at the rack). `0` is vanilla. This used to be a setting of the QoL fork (`qol.json`); keep that one at 0.

## Tactical carrier groups

`plane_group_per_launch` (default 3) groups AI carrier launches in normal tactical
combat, including allied AI. `plane_group_launch_delay_seconds` (default 0.5)
spaces the ready planes in game time. The stock six-second cooldown returns
after the group. Set group size to 1 for stock one-at-a-time launches. Size is
clamped to 1..64 and delay to 0.05..60 seconds. Stock deck-clearance and readiness
checks remain; blocked attempts cancel a partial group. Strategic air strikes
and manual launches keep their existing behavior. Restart after editing.

## TWR cutoff for dodging and tactics

Set `"twr_cutoff": 3.5` at the top level of `Modloader/config/hf_ai.json`,
alongside `ai_flares` and `tactics`, then restart the game. The default `0.0`
means no TWR restriction. A positive value suppresses tactics and missile
maneuvers for ships below that battle TWR; a ship exactly at the cutoff remains
eligible. Individual feature switches and normal target, timing and safety checks still apply; there are no separate acceleration gates.

Below the cutoff, ships do not sidestep missiles, roll, boost for evasion, retreat
or shelter during flare curtains, or react to stray missiles. They can still
deploy flares against locked-on missiles and during preventive curtains. Carrier
plane groups are independent of this setting.

## Deprecated settings

| Former key | Status since v0.10.4 | Use instead |
|---|---|---|
| `tactics.min_accel` | Ignored; remove from your JSON. Strafing and retreat remain available. | Top-level `twr_cutoff`. |
| `tactics.dive_min_accel` | Ignored; remove from your JSON. Dive avoidance and aim dodging remain available. | Top-level `twr_cutoff`. |
