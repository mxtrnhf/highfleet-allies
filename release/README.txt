HIGHFLEET ALLIES — PACK v@PACK_VERSION@
hf_ally @ALLY_VERSION@ + hf_ai @AI_VERSION@ + optional QoL build — Steam 1.163 only
Source: https://github.com/mxtrnhf/highfleet-allies
Research notes: https://github.com/mxtrnhf/hf_notes

===============================================================================
WHAT CHANGED SINCE PACK v0.14.18
===============================================================================
This is an ally-side configuration cleanup. hf_ai and QoL are unchanged.

- hf_ally @ALLY_VERSION@: the ally config is grouped by feature and no longer
  carries retired keys. Modloader/config/ally.json is the current preset.
- logging.debug_log is now the runtime on/off switch for debug mode and defaults
  to false. Debug mode still provides the megalog, watchdog, crash dump,
  pre-combat GUI lists, per-second probes and the lifted line budget. The legacy
  megalog cargo feature no longer forces it on.
- debug_overlay moved under logging (logging.debug_overlay, default false).
  logging.max_lines, logging.log_precombat, logging.probe_lines,
  logging.log_colors, logging.dead_node, plane_bombers, marker_seam,
  marker_delay_ms and max_enemy_ships are retired and ignored in old configs.
- ally_damage_color is written as HEX ("#00E4FF"); numeric values from older
  configs still load.
- max_enemy_ships was removed. It patched the strategic bomber scene, not normal
  tactical combat; vanilla tactical battles have no total enemy cap. Use
  enemy_group_size to control how many enemies are active at once. It controls
  the initial group and refills after the 30 s timer; enemy_all_at_once only
  sets that timer to 0.
- Retired settings and diagnostics are documented in the included docs.

===============================================================================
CONFIG COMPATIBILITY
===============================================================================
Updating from v0.14.21 (or earlier v0.14.x): replace
Modloader/config/ally.json with the file in this ZIP. The key order is grouped,
and the retired keys above are ignored if left in an old file.

The important value changes: logging.debug_log defaults to false,
debug_overlay lives under logging, ally_damage_color uses HEX, and
max_enemy_ships is gone. hf_ai.json is unchanged since hf_ai v0.10.11.

If you come from v0.14.13 or older, replace Modloader/config/hf_ai.json too
(or delete it so the mod generates a fresh preset). The v0.10.11 preset trims
hf_ai.json from 80 to 49 total leaf settings (48 gameplay settings plus
logging.log_panic), reworks universal aim dodging and early fire, merges dive
settings, and updates default ranges/boosts (launch_flares_evade_range=400,
dive_dwell_s=0.5, dive_boost=2.0, aim_boost=2.0). Keeping an old hf_ai.json
will feed stale values into the AI.
===============================================================================

INSTALL
1. Back up Saves.
2. Install logdot's Modloader v4:
   https://github.com/logdot/Highfleet-Modloader
3. Extract this ZIP directly into the folder containing Highfleet.exe. Merge
   Modloader/ and Media/, and remove previous hf_ally_v*.dll / hf_ai_v*.dll
   files from Modloader/mods/ so only ONE of each is present.
4. Use the included Modloader/config/ally.json as the new preset, or merge it
   into your existing custom profile. Do not keep the retired keys.
5. Keep the QoL flare crash fix enabled. The optional highfleet_qol.dll is our
   logdot QoL main + manual editor elevation and firing-arc rules build.

FILES AND VALIDATION
- Modloader/mods/hf_ally_v@ALLY_VERSION@.dll, hf_ai_v@AI_VERSION@.dll, highfleet_qol.dll
- Modloader/config/ally.json, hf_ai.json, logging.json, qol.full.json
- Media/Snd/RADIO/radio_stall_01.wav: our voice line, replacing that game file
- docs/: player-defaults, ally settings, retired settings/diagnostics, debug
  overlay, AI settings/tuning and QoL guide
- manifest.json: source commit, component versions and per-file SHA256 hashes
- Built by GitHub Actions without diagnostic features. In-game validation of the
  full pack remains the player's responsibility.

CREDITS
Built on logdot's Highfleet-Modloader, patchy-rs, highfleet-rs and highfleet-qol.
MIT licenses are included. Unofficial fan project; not affiliated with HighFleet's
creators or publisher. No base-game executable is included.
