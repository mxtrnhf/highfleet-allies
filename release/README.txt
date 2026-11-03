HIGHFLEET ALLIES — PACK v@PACK_VERSION@
hf_ally @ALLY_VERSION@ + hf_ai @AI_VERSION@ + optional QoL build — Steam 1.163 only
Source: https://github.com/mxtrnhf/highfleet-allies
Research notes: https://github.com/mxtrnhf/hf_notes

===============================================================================
!!! IMPORTANT: UPDATE YOUR Modloader/config/hf_ai.json WHEN INSTALLING !!!
===============================================================================
Do NOT keep your old hf_ai.json from earlier releases. Overwrite
Modloader/config/hf_ai.json with the new file from this ZIP (or delete your old
hf_ai.json so the mod generates a fresh one).
hf_ai v@AI_VERSION@ trims hf_ai.json from 79 keys to the 38 core settings,
reworks universal aim dodging and early fire, merges dive settings, and updates
default ranges/boosts (launch_flares_evade_range=400, dive_dwell_s=0.5,
dive_boost=2.0, aim_boost=2.0). Keeping an old hf_ai.json will feed stale
values into the new AI.
===============================================================================

WHAT CHANGED SINCE PACK v0.14.13
- UPDATE YOUR hf_ai.json: trimmed from 79 keys to 38 core gameplay settings;
  retired and internal keys are documented in docs/hf-ai-config-reference.md.
- Universal Aim Dodging (hf_ai v@AI_VERSION@): enemies dodge both player and
  ally gun aim, and allies dodge enemy gun aim. Picks the closest active
  hostile shooter, adds a 0.6 s player fire hold-over for single-click volleys,
  and rolls aim_chance (0.75) per staggered window to prevent endless juking.
- Reworked Early Fire (hf_ai v@AI_VERSION@): triggers offensively per window
  within aim_fire_range (1.6x gun reach) and checks gun alignment + friendly-
  fire corridor so AI ships do not shoot through allies.
- Flare Curtain & Dive Fixes (hf_ai v@AI_VERSION@): curtains wait until ~4 s
  before missile arrival before activating (no premature ceiling-flee at
  launch), check launch_flares_min_rack before every drop, and let ships
  without flare launchers still evade missiles. Declined dive rolls retry after
  dive_cooldown_s (2.0 s); dive_under settings merged into dive_chance/dwell.
- Engine Boosts & Allied Missile Dump (hf_ai v@AI_VERSION@): dive_boost and
  aim_boost now work even when ai_flares.enabled is false; allies now also use
  ai_missile_dump_*.
- Battle Debug Overlay (hf_ally v@ALLY_VERSION@): "debug_overlay": true in
  ally.json draws 250u/500u/1000u world-space rings, native AI state labels,
  AI steering waypoints with distance, and live hf_ai tactics/flare states.

INSTALL
1. Back up Saves.
2. Install logdot's Modloader v4:
   https://github.com/logdot/Highfleet-Modloader
3. Extract this ZIP directly into the folder containing Highfleet.exe. Merge
   Modloader/ and Media/, and OVERWRITE Modloader/config/hf_ai.json with the
   new preset in this pack. Keep only ONE hf_ally DLL and ONE hf_ai DLL in
   Modloader/mods; remove previous versions before loading the new ones.
4. Keep the QoL flare crash fix enabled. The optional highfleet_qol.dll is our
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
