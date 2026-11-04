# HighFleet Allies v@PACK_VERSION@

For **HighFleet Steam 1.163**.

> ## Config compatibility
> **Updating from v0.14.15 or v0.14.16? No config changes are required.** The `hf_ai.json` preset is unchanged since hf_ai v0.10.11.
>
> **Updating from v0.14.13 or older?** Copy the new `Modloader/config/hf_ai.json` from this ZIP over your existing one (or delete it so the mod generates a fresh preset). The v0.10.11 preset trims `hf_ai.json` from 80 to 49 total leaf settings (48 gameplay settings plus `logging.log_panic`), reworks aim dodging and early fire across all ships, merges dive settings (`dive_dwell_s: 0.5`, `dive_boost: 2.0`, `aim_boost: 2.0`, `launch_flares_evade_range: 400.0`), and retires flare-curtain sheltering. Keeping an older preset will override the tuned values with stale numbers.

## Why another update already?

**Fixed AI dive leading to calculated ground slam (expert move).**

Shortly after v0.14.16, a recording exposed dive denial trying to match the
altitude of a landed enemy. Its direct engine dash also bypassed native braking:
turning the requested direction horizontal did not remove downward momentum.
That deserves a hotfix rather than waiting for the next feature release.

## Changes since v0.14.16

### `hf_ai v@AI_VERSION@` — landed-target and tactical ground guards
- Dive denial now requires an airborne combat target. Landed, taking-off or unreadable target modes cancel an existing dive as well as preventing a new one.
- The entire tactical waypoint respects the configured ground-clearance floor; below it, tactics cannot add descent or weaken upward native avoidance.
- Tactical boost/dash stops near that floor so native braking and lift retain engine control. Failed waypoint composition no longer activates a tactical boost.
- Aim defense and early fire remain available against landed targets. No preset or config-key changes.
- `hf_ally v@ALLY_VERSION@` and QoL source are unchanged from the previous pack.

These are conservative fixed-reference guards, not terrain sampling or a guarantee
against all stock-AI ground collisions. Missile-panic/flare-curtain maneuvers are unchanged.

### Validation
- Windows release cross-build passes; this pack is built by GitHub Actions without diagnostic features.
- Clippy completes with warnings; formatting checks report existing crate-wide differences.
- Installed locally, but in-game validation of the fix remains pending. No new regression tests were added.

## Pack contents

`hf_ally v@ALLY_VERSION@`, `hf_ai v@AI_VERSION@`, and the bundled QoL build.

## Installation

1. Back up your `Saves` folder.
2. Extract the pack into the folder containing `Highfleet.exe`.
3. Remove any older `hf_ai_v*.dll` and `hf_ally_v*.dll` files from `Modloader/mods/` so only one of each DLL is present.
4. If you are coming from v0.14.13 or older, make sure you overwrite `Modloader/config/hf_ai.json` with the new one from this pack.
