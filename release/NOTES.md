# HighFleet Allies v@PACK_VERSION@

For **HighFleet Steam 1.163**.

> ## ⚠️ IMPORTANT: UPDATE YOUR `Modloader/config/hf_ai.json`!
> **Do not keep your old `hf_ai.json` when updating to this release.** Copy the new `Modloader/config/hf_ai.json` from this ZIP over your existing one (or delete your old `Modloader/config/hf_ai.json` so the mod generates a fresh one).
> `hf_ai v@AI_VERSION@` overhauls `hf_ai.json` (trimmed from 79 keys down to the 38 core settings), reworks aim dodging and early fire across all ships, merges dive settings (`dive_dwell_s: 0.5`, `dive_boost: 2.0`, `aim_boost: 2.0`, `launch_flares_evade_range: 400.0`), and retires flare-curtain sheltering. Keeping an old 79-key `hf_ai.json` will override the new tuned values with stale numbers.

## Changes since v0.14.13

### `hf_ai v@AI_VERSION@` — AI Rework & Config Cleanup
- **UPDATE YOUR `hf_ai.json`:** `hf_ai.json` has been cleaned up from 79 keys to the 38 core gameplay settings. Old internal math/timing keys are now retired to compiled defaults (and listed under Retired settings in `docs/hf-ai-config-reference.md`).
- **Universal Aim Dodging (`tactics.aim_*`):** Aim awareness now works across **all ships in battle** — enemies dodge both you and your allies, and your allies dodge enemies. When multiple shooters fire at the same ship, it dodges the closest active shooter's aim ray. Includes a `0.6 s` player fire hold-over so single-click cannon volleys register, and a per-ship staggered window with `aim_chance` (`0.75`) so AI-vs-AI fights don't turn into endless synchronized juking.
- **Reworked Offensive Early Fire (`tactics.aim_fire_*`):** AI ships now roll early fire offensively on their awareness window when their target is within `aim_fire_range` (`1.6x`) of gun reach (no longer only while being aimed at), with a gun-alignment and **friendly-fire corridor check** so they don't shoot through allies.
- **Preventive Flare Curtain Fixes (`ai_flares.launch_flares_*`):**
  - Curtains now wait until the missile is ~4 seconds from arrival before activating (`CURTAIN`), fixing premature cross-map retreats into the arena ceiling right at launch.
  - `launch_flares_min_rack` (`0.5`) is now checked before **every** flare drop so ships preserve their 50% flare reserve for close-range locked-on panic.
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
