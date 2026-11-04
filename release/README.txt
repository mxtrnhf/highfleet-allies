HIGHFLEET ALLIES — PACK v@PACK_VERSION@
hf_ally @ALLY_VERSION@ + hf_ai @AI_VERSION@ + optional QoL build — Steam 1.163 only
Source: https://github.com/mxtrnhf/highfleet-allies
Research notes: https://github.com/mxtrnhf/hf_notes

===============================================================================
CONFIG COMPATIBILITY
===============================================================================
Updating from v0.14.15 or v0.14.16: NO config changes are required. The hf_ai.json preset
is unchanged since hf_ai v0.10.11.

Updating from v0.14.13 or older: overwrite Modloader/config/hf_ai.json with the
new file from this ZIP (or delete your old hf_ai.json so the mod generates a
fresh one). The v0.10.11 preset trims hf_ai.json from 80 to 49 total leaf
settings (48 gameplay settings plus logging.log_panic), reworks universal aim
dodging and early fire, merges dive settings, and updates default ranges/boosts
(launch_flares_evade_range=400, dive_dwell_s=0.5, dive_boost=2.0,
aim_boost=2.0). Keeping an old hf_ai.json will feed stale values into the AI.
===============================================================================

WHAT CHANGED SINCE PACK v0.14.16
Fixed AI dive leading to calculated ground slam (expert move).

Why another update already? A recording exposed dive denial trying to match a
landed enemy's altitude, while direct dash thrust bypassed native braking.
This is a focused hotfix rather than waiting for the next feature release.

- hf_ai @AI_VERSION@: dive denial requires an airborne combat target; landed,
  taking-off or unreadable target modes cancel an existing dive too.
- Tactical waypoints respect the configured clearance floor. Near that floor,
  tactical boosts/dashes stop so native braking/lift keeps engine control.
- Aim defense and early fire still work against landed targets. No config changes.
- hf_ally @ALLY_VERSION@ and QoL source are unchanged from the previous pack.
- Fixed-reference guard, not terrain sampling or protection for all stock AI
  and missile-panic/flare-curtain movement. In-game validation remains pending.
- Windows cross-build passes. This pack uses GitHub Actions builds without
  diagnostic features. See NOTES.md for details.

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
