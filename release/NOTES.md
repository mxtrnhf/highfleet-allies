# HighFleet Allies v@PACK_VERSION@

For **HighFleet Steam 1.163**.

> ## Config compatibility
> **Updating from v0.14.15? No config changes are required.** The `hf_ai.json` preset is unchanged since hf_ai v0.10.11.
>
> **Updating from v0.14.13 or older?** Copy the new `Modloader/config/hf_ai.json` from this ZIP over your existing one (or delete it so the mod generates a fresh preset). The v0.10.11 preset trims `hf_ai.json` from 80 to 49 total leaf settings (48 gameplay settings plus `logging.log_panic`), reworks aim dodging and early fire across all ships, merges dive settings (`dive_dwell_s: 0.5`, `dive_boost: 2.0`, `aim_boost: 2.0`, `launch_flares_evade_range: 400.0`), and retires flare-curtain sheltering. Keeping an older preset will override the tuned values with stale numbers.

## Changes since v0.14.15

This is a maintenance release: both DLLs are rebuilt against the merged
`highfleet-rs` layouts, and `hf_ai` picks up the fixes developed after the last
pack. No new config keys and no major gameplay overhaul.

### `hf_ally v@ALLY_VERSION@` / `hf_ai v@AI_VERSION@` — shared highfleet-rs layouts
- Both mods are built against [highfleet-rs](https://github.com/logdot/highfleet-rs) revision `05c6207`, which now includes the Steam 1.163 aircraft/loadout, Body runtime, AircraftLogic, PartStats, Creature and GUI layouts.
- `hf_ally` drops its local `Plane`, `Loadout` and `Munition` copies; `hf_ai` now uses the library too. Shared offsets are checked at compile time, so an upstream layout change fails the build instead of silently reading the wrong field.
- Fields the library does not model yet stay local with documented raw offsets.
- **No behaviour or configuration change from this refactor.**

### `hf_ai v0.10.12` — native early-fire gate
- Offensive early fire now widens only the native group-0 range gate; it no longer writes player fire bytes. Native barrel alignment, friendly-fire, ammunition and burst-start checks remain.
- Native latched bursts can still outlast the one-second permission, so this is not a guarantee against friendly fire. Group 1 is unchanged.
- Defensive aim dodging no longer requires the defender to own a target.
- Shared engine-hook preparation survives flare-specific byte mismatches.
- Zero curtain start probability no longer freezes an existing curtain.

### `hf_ai v0.10.13` — dive and panic escapes dash
- Dive/aim dodges and boosted panic evasion now dash: both engine banks (`0x66`/`0x67`) and the swivel servos (`0x64`/`0x65`) are aimed along the escape direction at the configured boost, bypassing the stock AI velocity controller's `throttle x 25 x battle TWR` speed target.
- Tactic waypoints are 400 units, the throttle's saturation point, so the same dodge asks for full target speed instead of 71%.
- `panic_boost: 1.0` now fully disables the panic boost even when the shared engine hook is installed for tactics.
- Static-engine ships still rely on force-command projection; the dash is strongest on swivel-engine designs.

### Validation
- Windows release cross-build and the GitHub Actions build pass.
- In-game combat validation remains pending.

## Pack contents

`hf_ally v@ALLY_VERSION@`, `hf_ai v@AI_VERSION@`, and the bundled QoL build.

## Installation

1. Back up your `Saves` folder.
2. Extract the pack into the folder containing `Highfleet.exe`.
3. Remove any older `hf_ai_v*.dll` and `hf_ally_v*.dll` files from `Modloader/mods/` so only one of each DLL is present.
4. If you are coming from v0.14.13 or older, make sure you overwrite `Modloader/config/hf_ai.json` with the new one from this pack.
