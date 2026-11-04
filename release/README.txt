HIGHFLEET ALLIES — PACK v@PACK_VERSION@
hf_ally @ALLY_VERSION@ + hf_ai @AI_VERSION@ + optional QoL build — Steam 1.163 only
Source: https://github.com/mxtrnhf/highfleet-allies
Research notes: https://github.com/mxtrnhf/hf_notes

===============================================================================
CONFIG COMPATIBILITY
===============================================================================
Updating from v0.14.15: NO config changes are required. The hf_ai.json preset
is unchanged since hf_ai v0.10.11.

Updating from v0.14.13 or older: overwrite Modloader/config/hf_ai.json with the
new file from this ZIP (or delete your old hf_ai.json so the mod generates a
fresh one). The v0.10.11 preset trims hf_ai.json from 80 to 49 total leaf
settings (48 gameplay settings plus logging.log_panic), reworks universal aim
dodging and early fire, merges dive settings, and updates default ranges/boosts
(launch_flares_evade_range=400, dive_dwell_s=0.5, dive_boost=2.0,
aim_boost=2.0). Keeping an old hf_ai.json will feed stale values into the AI.
===============================================================================

WHAT CHANGED SINCE PACK v0.14.15
This is a maintenance release. No new config keys and no major gameplay overhaul.

- Shared highfleet-rs layouts (hf_ally @ALLY_VERSION@ / hf_ai @AI_VERSION@):
  both DLLs are rebuilt against highfleet-rs revision 05c6207, which now
  includes the Steam 1.163 aircraft/loadout, Body runtime, AircraftLogic,
  PartStats, Creature and GUI layouts. hf_ally drops its local Plane, Loadout
  and Munition copies; hf_ai now uses the library too. Shared offsets are
  checked at compile time, so an upstream layout change fails the build instead
  of silently reading the wrong field. Fields the library does not model yet
  stay local with documented raw offsets. No behaviour change from this.

- Native early-fire gate (hf_ai v0.10.12): offensive early fire now widens only
  the native group-0 range gate; it no longer writes player fire bytes. Native
  barrel alignment, friendly-fire, ammunition and burst-start checks remain.
  Native latched bursts can still outlast the one-second permission, so this is
  not a guarantee against friendly fire; group 1 is unchanged. Defensive aim
  dodging no longer requires the defender to own a target. Shared engine-hook
  preparation survives flare-specific byte mismatches, and zero curtain start
  probability no longer freezes an existing curtain.

- Dive and panic escapes dash (hf_ai v0.10.13): dive/aim dodges and boosted
  panic evasion now aim both engine banks (0x66/0x67) and the swivel servos
  (0x64/0x65) along the escape direction at the configured boost. This bypasses
  the stock AI velocity controller's throttle x 25 x battle TWR speed target.
  Tactic waypoints are 400 units (the throttle saturation point), so the same
  dodge asks for full target speed instead of 71%. panic_boost 1.0 now fully
  disables the panic boost even when the shared engine hook is installed for
  tactics. Static-engine ships still rely on force-command projection; the dash
  is strongest on swivel-engine designs.

- In-game combat validation remains pending. Windows release cross-build and
  the GitHub Actions build pass.

INSTALL
1. Back up Saves.
2. Install logdot's Modloader v4:
   https://github.com/logdot/Highfleet-Modloader
3. Extract this ZIP directly into the folder containing Highfleet.exe. Merge
   Modloader/ and Media/, and remove previous hf_ally_v*.dll / hf_ai_v*.dll
   files from Modloader/mods/ so only ONE of each is present.
4. If coming from v0.14.13 or older, overwrite Modloader/config/hf_ai.json with
   the new preset in this pack.
5. Keep the QoL flare crash fix enabled. The optional highfleet_qol.dll is our
   logdot QoL main + manual editor elevation and firing-arc rules build.

FILES AND VALIDATION
- Modloader/mods/hf_ally_v@ALLY_VERSION@.dll, hf_ai_v@AI_VERSION@.dll, highfleet_qol.dll
- Modloader/config/ally.json, hf_ai.json, logging.json, qol.full.json
- Media/Snd/RADIO/radio_stall_01.wav: our voice line, replacing that game file
- docs/: player-defaults, ally settings, debug overlay, AI settings/tuning and QoL guide
- manifest.json: source commit, component versions and per-file SHA256 hashes

CREDITS
Built on logdot's Highfleet-Modloader, patchy-rs, highfleet-rs and highfleet-qol.
MIT licenses are included. Unofficial fan project; not affiliated with HighFleet's
creators or publisher. No base-game executable is included.
