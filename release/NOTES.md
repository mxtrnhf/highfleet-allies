# HighFleet Allies v@PACK_VERSION@

For **HighFleet Steam 1.163**.

> ## ⚠️ IMPORTANT: UPDATE YOUR `Modloader/config/hf_ai.json`!
> **Do not keep your old `hf_ai.json` when updating to this release.** Copy the new `Modloader/config/hf_ai.json` from this ZIP over your existing one (or delete your old `Modloader/config/hf_ai.json` so the mod generates a fresh one).
> The preset introduced in hf_ai v0.10.11 trims `hf_ai.json` from 80 to 49 total leaf settings (48 gameplay settings plus `logging.log_panic`), reworks aim dodging and early fire across all ships, merges dive settings (`dive_dwell_s: 0.5`, `dive_boost: 2.0`, `aim_boost: 2.0`, `launch_flares_evade_range: 400.0`), and retires flare-curtain sheltering. Keeping an old preset will override the new tuned values with stale numbers. v0.10.12 and v0.10.13 use the same preset.

## Source follow-up: hf_ai v0.10.13

Not yet published as a release pack. Dive/aim dodges and the boosted panic evasion
now dash: both engine banks and the swivel servos are aimed along the manoeuvre's
escape direction at the boost, bypassing the stock AI's velocity controller and its
`throttle x 25 x battle TWR` speed target, so a boosted ship can outrun its own
speed target like a player holding Shift. Tactic waypoints are 400 units (the
throttle's saturation point) instead of 200. `panic_boost: 1.0` now really disables
the panic boost. Static-engine ships still rely on the force-command projection, so
the dash is strongest on swivel-engine designs. No in-game combat validation yet.

## Source follow-up: hf_ai v0.10.12

Not yet published as a release pack. Replaces forced fire-byte writes with a
native group-0 range-gate detour, retaining native barrel alignment, friendly-fire,
ammo and burst-start checks. Native latched bursts still bypass later checks;
this is not a guarantee against friendly fire or a fixed-duration burst.
Defensive aim dodging no longer needs an own target. Shared engine-hook preparation
survives flare-specific byte mismatches. Zero start probability no longer freezes
existing curtain state. No in-game combat validation yet.

## Changes since v0.14.13

### `hf_ai v@AI_VERSION@` — AI Rework & Config Cleanup
- **UPDATE YOUR `hf_ai.json`:** the preset was cleaned up from 80 to 49 total leaf settings (48 gameplay plus logging). Omitted internal math/timing keys still accept manual overrides (listed under Retired settings in `docs/hf-ai-config-reference.md`).
- **Universal Aim Dodging (`tactics.aim_*`):** Enemies dodge both you and your allies, and allies dodge enemies. Multiple threats use the closest active shooter's aim ray. Player fire-button state is sampled at 20 Hz with a `0.6 s` hold-over, not confirmed shot detection; brief clicks can be missed. Per-ship staggered windows use `aim_chance` (`0.75`).
- **Offensive Early Fire (`tactics.aim_fire_*`):** awareness rolls are offensive rather than tied to being aimed at. In v0.10.12 they grant one-second group-0 range permission up to `aim_fire_range` (`1.6x`) reach while preserving native firing checks, subject to the burst limitation above.
- **Preventive Flare Curtain Fixes (`ai_flares.launch_flares_*`):**
  - Curtains use a one-time ETA minus 4 s estimate, clamped to a 1–8 s wait before activating (`CURTAIN`), rather than retreating immediately on launch. Actual arrival time is not tracked.
  - `launch_flares_min_rack` (`0.5`) is checked before **every** salvo; the salvo can cross below the threshold, so retaining 50% is not guaranteed.
  - Ships without flare launchers still evade incoming missiles in all stages (curtain retreat, stray missile evasion, and close-range panic).
  - Retired `launch_flares_shelter` and `launch_flares_shelter_range`.
- **Dive Avoidance Fixes (`tactics.dive_*`):** Merged `dive_under_chance`/`dive_under_dwell_s` into `dive_chance` (`0.8`) and `dive_dwell_s` (`0.5`). A declined dive roll now retries after `dive_cooldown_s` (`2.0 s`) instead of latching off for the rest of the hover.
- **Decoupled Engine Boosts & Allied Missile Dump:** `dive_boost` and `aim_boost` now work even when `ai_flares.enabled` is `false`. Allied AI ships now also use Zenith burst dumping (`ai_missile_dump_*`).

### `hf_ally v@ALLY_VERSION@` — Battle Debug Overlay
- **New `debug_overlay` option in `ally.json` (default `false`):** Draws three world-space range rings (`250u`, `500u`, `1000u`) around every live ship, a ship-following label with the native AI driver state (`HOLD`, `ENGAGE`, `ENGAGE*`, `MOVE`, `LEAVE`, `PARKED`, `DEAD`, `ALARM`) and side/role (`P`/`A`/`E`/`N`), and the AI steering waypoint (`WP <dist>u` with a dotted lead-in and marker, or `DEST <dist>u` when off-screen).
- **Live `hf_ai` state feed:** With `hf_ai v@AI_VERSION@` loaded, the overlay adds a second `AI ...` line showing each ship's live `hf_ai` tactics decision (`STOCK`, `DIVE`, `DIVE-SKIP`, `AIM`, `PANIC`) and active flare/evasion mode (`CURTAIN`, `FLARES`, `EVADE`, `PANIC`, `ROLL`). Details in `docs/debug-overlay.md`.

## Pack contents

`hf_ally v@ALLY_VERSION@`, `hf_ai v@AI_VERSION@`, and the bundled QoL build.

## Installation

1. Back up your `Saves` folder.
2. Extract the pack into the folder containing `Highfleet.exe`.
3. **Make sure you overwrite `Modloader/config/hf_ai.json` with the new one from this pack** (and remove any older `hf_ai_v*.dll` and `hf_ally_v*.dll` files from `Modloader/mods/` so only one of each DLL is present).
