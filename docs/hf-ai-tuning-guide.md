# Tuning hf_ai: a plain-language guide (source: v0.10.13)

> **IMPORTANT — UPDATE YOUR `Modloader/config/hf_ai.json` WHEN UPGRADING TO v0.10.11+**
> When upgrading from before v0.10.11, replace your old `Modloader/config/hf_ai.json` with the new preset (or delete it so the mod generates fresh defaults). `v0.10.11` trims the preset from 80 to 49 total leaf settings (48 gameplay settings plus `logging.log_panic`), reworks aim dodging and early fire across all ships, merges dive settings, and retires flare-curtain sheltering. v0.10.12 and v0.10.13 keep that preset unchanged; no new release pack is published for these follow-ups.

Written for the person who tunes the AI, not for the person who reads the code. Every setting below is a key in `Modloader/config/hf_ai.json`; the full key reference (including retired/internal keys) is in [hf-ai-config-reference.md](hf-ai-config-reference.md).

## 1. Is "state machine" the right word?

Yes, with two clarifications:

* **Each AI ship has its own small state machine.** At any moment its tactical steering is in one state (`STOCK`, `DIVE`, `DIVE-SKIP`, `AIM`, `PANIC`), and its missile-defense layer is in `NONE`, `CURTAIN`, `FLARES`, `EVADE`, `PANIC`, or `ROLL` (visible live in battle if you turn on `"debug_overlay": true` in `ally.json`).
* **It is not a replacement brain.** The game's own AI runs every tick, picks targets, aims guns, and computes a desired *waypoint*. `hf_ai` sits on that output: it either leaves the stock waypoint alone (`STOCK`) or blends/replaces it when a maneuver triggers.

## 2. What happens every tick

1. **Gates.** `hf_ai` stays out (`STOCK`) unless the ship is AI-flown (never the ship you fly yourself), airborne (mode 1), the arena bounds are readable, and its side is enabled (enemies always; allies when `allies: true`). Since v0.10.12, defensive aim dodging needs no own target; dive avoidance and offensive early fire still require a living target other than itself.
2. **TWR gate.** Top-level `twr_cutoff` (default `2.0`) checks live strategic/Shipworks TWR (`creature +0x160c`, before UI rounding). Ships below the cutoff do not use tactical dodges or missile evasion maneuvers, though they can still drop flares. `0.0` disables the TWR gate.
3. **Priority order** (first match wins): **missile panic / curtain** > **aim dodge** > **dive avoidance** > **stock**.
4. **Steering composition.** The active maneuver blends its unit direction with the stock direction, then clamps against the ground (`ground_y`, `ground_clearance`, `ground_hard_y`) and arena walls (`edge_margin`).
5. **Engine boost and escape dash.** Maneuvers with a boost above `1.0` (`panic_boost`, `launch_flares_boost`, `dive_boost`, `aim_boost`) raise the AI's engine thrust commands like holding Shift. Since v0.10.13, dive/aim dodges and the boosted panic evasion also dash: both engine banks and the swivel servos are aimed along the manoeuvre's escape direction at the boost, which bypasses the stock AI's velocity controller. That controller brakes at a target speed of `throttle x 25 x battle TWR`, so a strength raise alone cannot make an AI ship outrun it. The flare-curtain retreat has no published direction and keeps the strength-only raise. Since v0.10.11, the engine-boost hook is installed whenever *any* boost is enabled, so `tactics` boosts work even if `ai_flares.enabled` is `false`.

## 3. The states and what each knob does

### 3.1 Missile panic (`ai_flares` block)
Triggers when a missile locked onto the ship comes within `trigger_range` (`400.0`), or — with `stray_missiles: true` — when an unguided/other-target missile's straight-line trajectory passes close within 3 seconds.
* A locked missile starts both a flare burst and evasion; a stray missile starts evasion only (a later lock adds a flare burst).
* Checked 10 times per second, rolling `reaction_probability` (`0.6`).
* Drops flares for `burst_seconds` (`1.5` s, ~10 flares per launcher), then enforces `min_interval_seconds` (`1.5` s) of cooldown after the panic ends.
* With `evade: true`, the ship steers sideways/upward away from the missile's approach line for `evasion_seconds` (`3.0` s) with blend `evasion_strength` (`0.5`). Ships **without** flare launchers still execute full evasion maneuvers.
* Rolled once per panic: `panic_evasion_probability` (`0.1`) triggers a full-course escape: one committed direction, full thrust along it at `panic_boost` (`2.0`), for `panic_evasion_seconds` (`5.0` s). `barrel_roll_probability` (`0.001`) triggers a `barrel_roll_seconds` (`2.0` s) spin.

### 3.2 Preventive flare curtain (`launch_flares_*` in `ai_flares`)
When the opposing side launches a missile, eligible AI ships schedule a pre-emptive flare curtain before the missile locks on:
* **Who reacts:** The ship the missile is aimed at rolls `launch_flares_target_probability` (`1.0`). With `launch_flares_targeted_only: true`, ships within 600 units of the target roll `launch_flares_probability` (`0.6`); with `false`, all ships on that side roll `launch_flares_probability` while the target still uses `launch_flares_target_probability`.
* **Timing:** Flares last 5 seconds in the air. At scheduling, a one-time distance / assumed missile speed (`230u/s`) estimate gives a wait of ETA minus 4 s, clamped to **1–8 s** before activation (`CURTAIN`). This does not track the actual impact time; unknown ETA uses a random 1–2 s wait.
* **Flare budget:** Once active, drops occur every `0.5..1.0` s up to `launch_flares_max_drops` (`6`). `launch_flares_min_rack` (`0.5`) is checked **before every salvo** against observed peak capacity. A salvo can take ammunition below that threshold, so a 50% remainder is not guaranteed.
* **Retreat:** While active and within `launch_flares_evade_range` (`400.0`), the ship steers away from the incoming missile (`launch_flares_evade: 0.9`, `launch_flares_boost: 2.0`), even without flare launchers or below the reserve threshold. Start probability does not suspend an existing curtain's expiry, panic cancellation or drops.

### 3.3 Dive avoidance ("deny the dive", `tactics.dive_*`)
If a ship's target stays within 45° of straight below (`dive_under: true`) or straight above (`dive_over: true`), closer than `dive_range` (`600.0`), for `dive_dwell_s` (`0.5` s), the ship rolls `dive_chance` (`0.8`):
* **Success (`DIVE`):** Sidesteps horizontally (picking the wrong-foot direction against a fast-moving target, or away from a nearby arena wall) while pulling toward the target's altitude, dashing along that sidestep at `dive_boost` (`2.0`), for `1.5..3.0` s, followed by `dive_cooldown_s` (`2.0` s).
* **Declined (`DIVE-SKIP`):** Since v0.10.11, a failed roll pauses for `dive_cooldown_s` (`2.0` s) and then re-evaluates if the target is still hovering in the cone, instead of latching off forever.

### 3.4 Universal aim dodging & offensive early fire (`tactics.aim_*`)
Reworked in v0.10.11 so **all sides dodge each other's gun aim**:
* **Staggered per-ship windows:** Each AI ship runs its own awareness clock (`1.5` s window every `2.5..3.5` s) rather than a single global clock.
* **Universal threat rays:** During an open window, the ship checks all hostile live ships (enemies check the player **and** your allies; your allies check all enemies). For the player, aim comes from the mouse aim vector (`0x1403c0dc8`); for AI ships, aim comes from `AiWeaponController`'s lead vector (`creature +0xbf8/+0xbfc`).
* **Multiple shooters:** If both you and an ally (or multiple enemies) aim at the same ship within `aim_range` (`1800.0`) and `aim_tolerance` (`200.0`), it dodges the **closest active shooter's** ray.
* **Firing check (`aim_require_fire: true`):** Player fire-button state is sampled at 20 Hz and held for `0.6` s after a firing sample. This is not confirmed shot detection; very brief clicks can be missed. AI shooting detection checks active gun rate timers and burst latches.
* **Anti-juking roll (`aim_chance: 0.75`):** Rolled once per awareness window when a hostile ray crosses the ship, so AI-vs-AI fights don't become endless 100% juking matches.
* **Dodge dash (`aim_boost: 2.0`):** While the slip is active the ship aims both engine banks and the swivel servos along the slip direction at `aim_boost`, so it dashes out of the line instead of only pushing harder inside the AI's speed controller. `1.0` turns the dash off.
* **Offensive early fire (`aim_fire_probability: 0.5`, `aim_fire_range: 1.6`):** Once per awareness window in range, the ship rolls for 1 s of widened **group-0 range permission**. Since v0.10.12, the native weapon controller still requires actual barrel alignment (0.025 radians), its friendly-fire gate, ammunition and normal burst-start eligibility. The native predicted aim-point distance must be within `1.6x` group-0 reach; group 1 and ordinary stock fire are unchanged. No player fire byte is written. **This is not a friendly-fire guarantee:** native already-latched bursts bypass later alignment/corridor checks and may continue past the permission window. Permission is rejected for stale identities, control handoffs, lost/changed targets or stale steering.

## 4. Ally support (`v0.10.11`)

| Feature | Allies (`allies: true`) |
|---|---|
| Locked & stray missile panic / flares | Yes (`ai_flares.allies`) |
| Preventive flare curtain (on enemy launches) | Yes (`ai_flares.allies`) |
| Dive avoidance | Yes (`tactics.allies`) |
| Aim dodging (against enemy gun aim) | Yes (`tactics.allies`) |
| Offensive early fire (retains native firing checks; not a no-friendly-fire guarantee) | Yes (`tactics.allies`) |
| Zenith missile burst dump (`ai_missile_dump_*`) | Yes (`ai_flares.allies` or `tactics.allies`) |
| The ship you fly yourself | Never touched |

## 5. Quick tuning recipes

| What you see in battle | What to change in `Modloader/config/hf_ai.json` |
|---|---|
| Neighbouring ships drop too many preventive flares | Keep `launch_flares_targeted_only: true` and lower `launch_flares_probability` (e.g. `0.3` or `0.0`), leaving `launch_flares_target_probability: 1.0` so only the targeted ship drops a curtain. |
| AI ships dodge my cannon shots too often | Lower `tactics.aim_chance` (default `0.75` -> `0.4`) or narrow `tactics.aim_tolerance` (`200.0` -> `120.0`). |
| AI ships never dodge until I'm already shooting | Set `tactics.aim_require_fire: false` so they react to hostile aim lines even before the trigger is pulled. |
| Ships still let me camp directly underneath them | Lower `tactics.dive_dwell_s` (`0.5` -> `0.25`) or raise `tactics.dive_chance` (`0.8` -> `1.0`). |
| Enemies or allies open fire from too far away | Lower `tactics.aim_fire_range` (`1.6` -> `1.25`) or set `tactics.aim_fire_probability: 0.0`. |
| Heavy ships never dodge | Lower top-level `twr_cutoff` (`2.0` -> `1.5`, or `0.0` to allow all ships). |

## 6. Retired settings

Moved out of the default `hf_ai.json` in v0.10.11 (see [hf-ai-config-reference.md](hf-ai-config-reference.md#retired-settings) for full details and ranges):

* **Retired feature keys (ignored):**
  * `ai_flares.launch_flares_shelter`, `ai_flares.launch_flares_shelter_range` — sheltering behind another ship's flare curtain was removed; ships without flares still evade/retreat from the missile directly.
  * `tactics.dive_under_chance`, `tactics.dive_under_dwell_s` — merged into `tactics.dive_chance` (`0.8`) and `tactics.dive_dwell_s` (`0.5`).
* **Internal tuning keys (omitted from `hf_ai.json` for clarity, but still accepted if added manually):**
  * `ai_flares`: `launch_flares_interval_min` (`0.5`), `launch_flares_interval_max` (`1.0`), `launch_flares_seconds` (`5.0`), `launch_flares_cooldown` (`6.0`), `launch_flares_neighbor_range` (`600.0`), `launch_flares_delay_min` (`1.0`), `launch_flares_delay_max` (`2.0`), `launch_flares_eta_lead` (`4.0`), `launch_flares_eta_max_wait` (`8.0`), `launch_flares_missile_speed` (`230.0`), `stray_missile_radius` (`150.0`), `stray_missile_eta_s` (`3.0`), `stray_missile_range` (`1200.0`).
  * `tactics`: `ground_y` (`200.0`), `ground_clearance` (`450.0`), `ground_hard_y` (`120.0`), `edge_margin` (`300.0`), `dive_cone_deg` (`45.0`), `dive_min_s` (`1.5`), `dive_max_s` (`3.0`), `dive_weight` (`8.0`), `dive_wrongfoot_speed` (`40.0`), `dive_level_weight` (`0.7`), `aim_window_s` (`1.5`), `aim_gap_min_s` (`2.5`), `aim_gap_max_s` (`3.5`), `aim_weight` (`12.0`), `aim_fire_s` (`1.0`).
* **Retired in v0.10.4–v0.10.6 (ignored):**
  * `tactics.min_accel`, `tactics.dive_min_accel` (replaced by top-level `twr_cutoff`).
  * `tactics.k_center`, `band_min`, `band_max`, `half_width`, `outrange_extra`, `blend_weight`, `strafe_min_s`, `strafe_max_s`, `retreat_tangent`, `strafe_enabled` (optional positioning removed in v0.10.6).
