# hf_ai

A tactical AI layer for Highfleet (Steam 1.163), standalone DLL for logdot's Highfleet-Modloader.

**Approach:** keep the stock per-ship AI tick (`FUN_140062f90`) and override its outputs (steering waypoint
`creature+0xbf0/0xbf4` written at `0x140064a8d`, engine command strength at `0x140065744`, fire bytes, launch gating)
from a small set of tactics. Tactic logic is code; its numbers (ranges, timings, weights) live in
`Modloader/config/hf_ai.json` (was `ai.json` in v0.1.0, which was never installed), to be hot-reloaded while the game runs.

**Data to design from:** `hf_ally` telemetry builds (`--features telemetry`), see `hf_notes/docs/findings/ai-telemetry.md`.

Build: `cargo xwin build --release --target x86_64-pc-windows-msvc`.
Tests: `cargo xwin test --release --target x86_64-pc-windows-msvc` (runs under wine); the pure math alone:
`rustc --edition 2021 --test src/panic_math.rs -o /tmp/pm_test && /tmp/pm_test`.

Status: **v0.10.0** (adds the AI missile dump moved from the qol fork). Earlier: v0.7.1 was played (tactics: deny the dive, aim awareness, early fire, stray avoidance; ai_flares: ETA-timed flare curtain with shelter and retreat). Versions and config: `hf_notes/docs/mods/hf-ai.md`.

## Stages in the steering hook (`0x140064a8d`, every AI tick of every ship)
1. the stock AI's waypoint (200 units from the ship along the wanted direction);
2. `tactics::decide(ctx)`: the tactics stage ("strafe in the band", v0.3.0, off by default, see below);
3. the ported missile panic (`ai_flares::steer`): while a ship is evading it replaces the waypoint, so **panic overrides tactics**;
4. the CSV row (`Modloader/logs/hf_ai_steer_<unix>.csv`, `log_steering`, `log_hz`); `chosen_x/y` is the final waypoint,
   `panic` = 1 when stage 3 chose it, `cptr` = creature pointer, `state` (last column) = stock / far / band / close / panic; `dist` is the range to the target;
5. the waypoint goes back to the tick.

## `ai_flares` (ported from the qol fork, behaviour-identical)
The qol fork's `ai_flares` (branch `ai-flares`, `src/ai_flares.rs`, commit 5c3ab73) lives in `src/ai_flares.rs`
(hooks, game-memory reads), `src/panic_math.rs` (pure timers and direction math, host-testable), `src/flares_config.rs`
(the config block, `sanitized()` unchanged), `src/rng.rs` (copy). What it does is described in
`hf_notes/docs/findings/ai-flares.md` and `ai-hf-ai-design.md`: per AI ship, scanned every 0.1 s, a missile locked on to
it inside `trigger_range` starts a panic with probability `reaction_probability`: a flare burst (hook `0x14007f864`), a
sidestep evasion (steering), sometimes a boosted panic evasion with one committed direction, rarely a barrel roll
(engine commands, hook `0x140065744`). It uses the missile position only (no velocity, no time to impact).

**Config** (`Modloader/config/hf_ai.json`, written with defaults when missing; every key optional):
```json
"ai_flares": {
  "enabled": false,
  "trigger_range": 400.0, "reaction_probability": 0.6,
  "burst_seconds": 1.5, "min_interval_seconds": 1.5,
  "evade": true, "allies": true,
  "evasion_seconds": 3.0, "evasion_strength": 0.9,
  "barrel_roll_probability": 0.01, "barrel_roll_seconds": 2.0,
  "panic_evasion_probability": 0.3, "panic_boost": 2.0, "panic_evasion_seconds": 8.0,
  "log_panic": false
}
```
`log_panic` writes `Modloader/logs/hf_ai_panic_<unix>.log`: one line per event (START with mode / missile distance /
missile and ship position / chosen side / boost / timers, EXTEND, IGNORED, END, FLIGHT, STEER and BOOST/SPIN rate
limited to 2 per second per ship). Columns `t` and ship pointer join with the steer CSV and with `hf_ally` telemetry.

**Rules that must hold**
* **Exactly one** of qol `ai_flares.enabled` and hf_ai `ai_flares.enabled` may be on. Both patch `0x14007f864`,
  `0x140064a8d` and `0x140065744`; the mod that loads second finds other bytes and silently skips (hf_ai logs an error line
  naming the site). Today qol has `ai_flares.enabled: true`: turn it off in `qol.json` first, then set hf_ai's to `true`.
* hf_ai's steering hook is **always** installed (log-only when its `ai_flares` is off). So with hf_ai in the game, qol's
  `ai_flares.evade` can never work, and the qol panic boost (which needs the steering hook) goes with it. Do not leave qol
  `ai_flares` on "for the flares only" together with hf_ai: flare site and boost site would be fine for qol but its
  evasion and boost are not.
* qol `enable_flare_crash_fix` must stay **on**. AI flares put more flares in play and the proximity-fuze crash is fixed
  only by that qol patch. hf_ai checks the site at the first panic and logs a warning if it still has the original bytes.
* `ai_missile_dump_chance` / `ai_missile_dump_min_delay` / `ai_missile_dump_max_delay` (v0.10.0, moved here from the qol fork; sites `0x140065edb` / `0x140065c6c`): chance that an enemy ship keeps launching Zeniths right after a launch. 0 = vanilla.

**Extension point:** `panic_math::scan`, marked `EXTENSION POINT (threat decision)`: the found threat carries the missile
body address, position and distance; time-to-impact or overshoot logic goes between the threat and the reaction roll.
Not implemented; v0.2.0 only reproduces qol.

**Safety:** all site bytes are checked before anything is prepared; one `PatchSession` installs steering, flare and
boost hooks together or none; game-memory reads are page-checked (`VirtualQuery`, region-cached in the body scan); body
and launcher loops are capped; the three hooks run inside `catch_unwind`; the ship table lock recovers from poisoning.

## `tactics`: strafe in the band (v0.3.0, off by default)
Code: `tactics_math.rs` (pure, host-testable: `rustc --edition 2021 --test src/tactics_math.rs`), `tactics.rs` (game reads,
per-ship table), `tactics_config.rs` (the `tactics` block). Spec: `hf_notes/docs/findings/ai-hf-ai-design.md`.

**Gates (any failure = stock, nothing changes):** `tactics.enabled`; AI-flown (`+0xbd4 != 0`); enemy side (`+0xad4 < 0`; allies
only with `apply_to_allies`); airborne combat mode (`+0xb48 == 1`; landed 3 and takeoff 0 stay stock; this is the telemetry
`mode` column, NOT `+0xd70` which is the role); a living target ship; `accel_est = (+0x1664 + +0x1668) / +0x15b8 >= min_accel`;
not in a missile panic; arena bounds readable and sane.
**Band:** centre = `k_center` x own reach (max of `+0x1838/+0x183c`, the largest value seen per ship, because AP shells
drop it to 500), limited to `band_min..band_max`; +`outrange_extra` if the target's reach is longer. Band = centre +- `half_width`.
**States by range d:** far (d > outer) = stock; band = tangential strafe (side reverses after a random `strafe_min_s..strafe_max_s`);
close (d < inner) = retreat on a diagonal (away + `retreat_tangent` x tangent). 40 u hysteresis on both edges.
**Output:** `ship + 200 * normalize(unit(stock) + blend_weight * tactic_dir)`, length 200 like stock. Safety limits are applied to
that final direction: near the ground (`ship.y > ground_y - ground_clearance`; y points DOWN) never more downward than
`max(stock.y, 0)`; within `edge_margin` of an arena edge never more outward than the stock direction; stock and tactic nearly
cancelling (blend shorter than 0.2) or an unmeetable limit = stock. Arena bounds are read live from `0x1403c2bc0..` (x max, y max, x min, y min).
Panic overrides tactics (stage 3). Per-ship state is cleared when the game clock goes backwards or jumps more than 5 s, entries
not seen for 2 s are dropped. No hot reload: edit the config and restart the game.

```json
"tactics": {
  "enabled": false, "apply_to_allies": false, "min_accel": 35.0,
  "k_center": 0.32, "band_min": 300.0, "band_max": 550.0, "half_width": 100.0, "outrange_extra": 150.0,
  "blend_weight": 1.0, "strafe_min_s": 2.0, "strafe_max_s": 5.0, "retreat_tangent": 0.7,
  "ground_y": 200.0, "ground_clearance": 450.0, "edge_margin": 300.0,
  "strafe_enabled": true,
  "dive_enabled": true, "dive_chance": 0.8, "dive_cone_deg": 45.0, "dive_range": 600.0, "dive_dwell_s": 1.0,
  "dive_min_s": 1.5, "dive_max_s": 3.0, "dive_cooldown_s": 2.0, "dive_weight": 4.0, "dive_min_accel": 30.0,
  "dive_under": true, "dive_over": true, "dive_boost": 2.0, "dive_wrongfoot_speed": 40.0
}
```

### Flare curtain (v0.5.0, `ai_flares` block, off by default)
When the player's side launches a missile, each enemy ship rolls `launch_flares_probability` ONCE per launch; on a hit it drops flares
every `launch_flares_interval_min..max` s (0.5..1.0) for `launch_flares_seconds` (5), BEFORE any lock-on, like a helicopter flooding
the area with fake heat. ALL launchers fire on every drop (each obeys its own rate gate), at most `launch_flares_max_drops` (6) so the racks are not empty when
the real panic needs them (v0.5.1: every drop fires ALL launchers at once); a further launch extends it; `launch_flares_cooldown` (6 s) after it ends. A real panic cancels it and
its timers are never touched. Enemy ships only. A launch is a live missile body whose target link points at an enemy ship (the
game launches only at the owner's target, so a target always exists). The first 12 launches write a diagnostic line to
`Modloader.log` with the target / target-owner / missile-owner sides (the owner link at body+0x4b0 is inferred, not used). Trace
events with `log_panic`: `CURTAIN_START/DROP/EXTEND/END`. Needs `ai_flares.enabled`.
```json
"ai_flares": { "launch_flares_probability": 0.6, "launch_flares_interval_min": 0.5, "launch_flares_interval_max": 1.0,
               "launch_flares_seconds": 5.0, "launch_flares_max_drops": 6, "launch_flares_cooldown": 6.0 }
```

v0.5.2: while a curtain runs the ship also moves AWAY from the launched missile (`launch_flares_evade` 0.9 = share of its course taken over, 0 = off; `launch_flares_evade_range` 1800 u: nearest missile of the player's side within that range; retreat plus a 0.35 sidestep to the curtain's random side; never downward when the ship is below y=-250). It goes through the same steering hook as the panic, so the steer CSV `state` column says `panic` for those rows. A real panic overrides it.

v0.5.3: the retreat also boosts the engines (`launch_flares_boost` 2.0, 1.0 = off) through the same engine hook as the panic boost, held 0.3 s per steering decision.

v0.5.4 (after the v0.5.x analysis): the curtain only DROPS flares while the racks hold at least `launch_flares_min_rack` (0.5) of the most flares the ship was seen to carry, and never for a ship without flare launchers; those still run the curtain's retreat (and boost). Retreat waypoints are clamped into the arena (50 u margin at the sides and ceiling) with the bounds the tactics stage reads.

v0.5.5: preventive defence. A curtain starts only for the ship the launched missile is aimed at, or a ship within `launch_flares_neighbor_range` (600 u) of it (`launch_flares_targeted_only` true; false = every enemy rolls again). The target is the missile's target link (or its owner when the link points at a part); ships that are not eligible just ignore that launch. Only the newest launch's target counts.

### Aim awareness (v0.6.0, `tactics.aim_enabled`, off by default)
The player sees where the enemy guns point; the AI had no such data. The player's aim is two floats at `0x1403c0dc8/dcc`
(world-aligned direction, y not flipped; re-checked 2026-10-18 on session 1792336456: median 7 deg to the nearest enemy while
firing, 24 deg while not). A SHARED awareness window opens for `aim_window_s` (1.5) at random start-to-start gaps of `aim_gap_min_s..aim_gap_max_s`
(start to start, 2.5..3.5 s). During a window every enemy that targets the player and sits within `aim_tolerance` (200 u) of the aim ray, ahead of
the player and within `aim_range` (1800 u), slips off the line (perpendicular to the aim, to the side it is already on) with
`aim_weight` (8) and `aim_boost` (1.5). Ships off the line, behind the player, or not targeting the player are not affected (so
aiming at ship A does not alarm ship B unless the line crosses B too). State `aim` in the steer CSV; gates and `dive_min_accel` as
the dive; panic wins; it takes priority over the dive. Not done: early fire (needs the AI weapon controller's fire gate, see notes).

v0.6.1 early fire (`aim_fire_probability`, default 0 = off; `aim_fire_s` 1.0; `aim_fire_range` 1.6): once per awareness window, a ship on the aim line rolls the chance; on a hit it writes 1 into its group-0 fire byte (creature +0xbbc, the player's LMB; the AI weapon controller never uses it, CreatureUpdate then calls FireGuns for every gun of the group and each gun obeys its own reload) for `aim_fire_s`, only while the player is within `aim_fire_range` x its gun reach. The byte is cleared after the window whatever early return the decision took. Bypasses the vanilla range (1.0 x reach), 0.025 rad angle and friendly-fire tests (the guns track the target continuously, so the angle is usually right; allies near the line of fire can be hit). The telemetry column `f0` of the enemy rows shows the byte.

v0.6.2 shelter (`launch_flares_shelter` true, `launch_flares_shelter_range` 1200): a curtain ship that never had a flare (no launcher) does not just run from the missile; if an ally within the range is dropping flares it heads for the point 250 u BEHIND that ally as seen from the missile (the flares are between the missile and the sheltering ship), blended with its own course by `launch_flares_evade`, with the same boost. No dropping ally in range: the old run-away retreat.

v0.6.3 (after the v0.6.0 analysis: aim dodge moved ships only ~17 u/s, 24% of on-line time the player was not even shooting, a diving Sarma died on the ground): `aim_require_fire` (true: the aim dodge only happens while the player's fire byte of either gun group is set), `aim_weight` 8 -> 12, `aim_boost` 1.5 -> 2.5, `dive_boost` 2.0 -> 2.5, and `ground_hard_y` (120): below that y no tactic steers downward at all, whatever the stock direction does.

v0.6.4 panic evasion fix (reported on session 1792339104, t=61.32: a Ballistic at (73,-49), ~300 u above the ground, evaded a missile from the upper left by flying straight DOWN, with the flares falling below it): of the two sidestep sides the HIGHER one now wins whenever they differ by more than 0.15 in the unit direction (`side` only breaks ties, e.g. a missile straight above), and a ship below y=-250 (the ground is about +250) never steers downward during a panic evasion (`NO_DESCENT_BELOW_Y` in `panic_math.rs`). This also applies to the panic-evasion flight direction. The four qol-original evasion tests were moved to a high ship because the side is no longer honoured blindly.

v0.7.0: (1) curtain delay `launch_flares_delay_min/max` (1.0..2.0 s): the first drop waits so the flares are still in the air when the missile arrives; the curtain lasts `launch_flares_seconds` after the delay. (2) STRAY MISSILES (`stray_missiles` true, `stray_missile_radius` 150 + half the ship's size, `stray_missile_eta_s` 3, `stray_missile_range` 1200): a guided missile that is NOT locked on a ship (lost lock, or aimed at someone else; never the ship's own missile, owner link +0x4b0) counts as a threat for the panic (flares, evasion, boost; same reaction roll) when its straight-line path (velocity from successive positions, 10 Hz) passes within the radius of the ship within the ETA. Pure maths `panic_math::stray_cpa`.

v0.7.1 (after the v0.6.4 analysis: flares live exactly 5.0 s, the missile arrives 3-9 s after the launch, 48% of drops were gone when it arrived; 10 of 15 targeted ships had no launcher; the two flare ships whose 60% roll failed were both hit; shelter never acted): the first drop is now timed by the missile ETA (distance / `launch_flares_missile_speed` 230) as `ETA - launch_flares_eta_lead` (4 s), between `launch_flares_delay_min` and `launch_flares_eta_max_wait` (8 s), the fixed random delay is only the fallback; the ship the missile is aimed at rolls `launch_flares_target_probability` (1.0), neighbours keep `launch_flares_probability`; the retreat/shelter range is 3000 u (was 1800), so it starts at the launch; any curtain ship that cannot drop (no launcher OR racks under the reserve) shelters behind a dropping ally.

Unreleased: a stray/lost-lock missile now starts **evasion only**, without a flare burst. If a locked missile appears while that evasion runs, the ship adds one flare burst; an expired burst is not restarted on every scan.

### Deny the dive (v0.4.0)
v0.5.4: a target BELOW the ship ("under", the damaging case) has its own `dive_under_chance` (1.0) and `dive_under_dwell_s` (0.5 s); `dive_chance` / `dive_dwell_s` now apply to a target above. The dodge adds a pull onto the target's altitude (`dive_level_weight` 0.7, full strength from 150 u of height difference): the ship tries to get onto your level so the keel cannot be shot at. `dive_weight` default 4 -> 8 (the dodge started too slowly).
v0.5.1: the dodge side is chosen AGAINST the target's sideways motion (`dive_wrongfoot_speed` u/s, default 40; 0 = random): the player has to reverse his momentum to follow. A near arena wall still wins.
If the target hangs within `dive_cone_deg` of straight below (`dive_under`) or above (`dive_over`) the ship, within
`dive_range`, for `dive_dwell_s`, the ship rolls once (`dive_chance`, default 80%) per time the target enters the cone. On a hit it
sidesteps horizontally for a random `dive_min_s..dive_max_s` (away from a near arena wall, else a random side; same safety limits
as the strafe: ground, arena edges), blended with the stock direction at `dive_weight`. Then a `dive_cooldown_s` pause. On a miss it
stays stock until the target leaves the cone. Panic overrides it. CSV `state` column: `dive`, `dive_skip` (declined by the roll).
`dive_enabled` and `strafe_enabled` are separate switches (`enabled` is the master): dive-only = `strafe_enabled:false`; strafe-only =
`dive_enabled:false`. The gate uses the lowest threshold of the enabled tactics, each tactic re-checks its own (`min_accel` 35 strafe,
`dive_min_accel` 30 dive; 37 of 159 designs are below 35, heavies like Gryphon/Novorossiysk). Known limits: only `ctx.target` counts
(an enemy aiming at an ally ignores you under it); hull rotation is not controlled (waypoint only).
A v0.2.0 `hf_ai.json` without a `tactics` block loads with tactics off. The log (`Modloader/Modloader.log`) gets one line with the
arena bounds per battle and, for the first 24 ships, `accel_est` next to `twr*9.82` (they should match: Lightning ~128, Intrepid ~56).

**Install checklist:** back up `hf_ai.json`; qol `ai_flares.enabled=false` and hf_ai `ai_flares.enabled=true` in the same step
(only if flares are wanted); `tactics.enabled=true` only for the test; qol `enable_flare_crash_fix` stays on.
**First in-game check (blocks all tuning):** `decide()` returning a waypoint has never run. Fly one fight with tactics on and
confirm in the `hf_ai_steer_*.csv` (rows with `state` band/close and `chosen_x/y` set) and the `hf_ally` telemetry (`rp_*`
velocity) that the ship's velocity follows `chosen` within about 1 s; check the logged `accel_est` vs `twr*9.82` and the bounds line.
Then A/B tactics on/off against the same enemy.

**Known tuning risk:** the stock AI often already orbits the target in the band (stand-off 0.35 x reach, orbit weight
1 - |radial|). With `blend_weight` 1.0 a strafe side opposite to the stock orbit cancels it (blend < 0.2 = stock, no override),
so the reversal may barely show. First A/B knob: `blend_weight` 2.0. Test `orbiting_stock_vs_strafe_side_weight_matters`.
