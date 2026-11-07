HIGHFLEET ALLIES — PACK v@PACK_VERSION@
hf_ally @ALLY_VERSION@ + hf_ai @AI_VERSION@ + optional QoL build — Steam 1.163 only
Source: https://github.com/mxtrnhf/highfleet-allies
Research notes: https://github.com/mxtrnhf/hf_notes

===============================================================================
WHAT CHANGED SINCE PACK v0.14.22
===============================================================================
First public pack since v0.14.22. hf_ai and QoL are unchanged.

ally.json changes
- enabled is the only master switch. spawn_ally and ally_toggle are retired
  and ignored. When enabled, the wingman deploys and the pre-combat
  ALLY/plane-loadout UI is installed; use the per-ship ALLY toggle to skip the
  wingman.
- ally_reticles is retired; ally_reticle_scale controls size and visibility.
  0 = off; positive = size; negative/non-finite fall back to 0.8.
- max_enemy_ships is retired (it patched the strategic bomber scene, not
  normal tactical combat). Use enemy_group_size for enemies active at once.
- plane_bombers, marker_seam, marker_delay_ms and the retired logging keys
  (logging.max_lines, logging.log_precombat, logging.probe_lines,
  logging.log_colors, logging.dead_node) are ignored. debug_overlay lives
  under logging.
- ally_damage_color is HEX ("#00E4FF"); numeric values still load.
- The ALLY and plane-loadout buttons use a per-button 120 ms debounce.
- The pack's Modloader/config/ally.json is the current grouped preset with no
  retired keys.

Ally radio lines follow the game language
- Russian plays from Media/Snd/RADIO; English plays from Media/Snd/RADIO_ENG.
  A language change in the options applies to the next line.
- The pack keeps the existing custom Russian
  Media/Snd/RADIO/radio_stall_01.wav; English uses the stock English line.
- Custom voice paths and custom audio for ally lines are planned for a future
  version; they are not in this pack.
- The pre-combat button_01 click is unchanged and is not localized.

Battle honor / XP sharing
- New ally_honor_share (default 0.5). The player keeps 100% of the honor/XP
  earned; every deployed ally gets 50% of the player's award as it is earned.
- 0 disables sharing; 1 gives allies the full award; values outside 0..=1 are
  clamped; non-finite values use 0.5.
- Uses the native battle honor field; the game converts it to each ship's
  persistent m_xp, stars and promotions after the battle.
- Late-joining allies share from deployment; a ship that leaves stops earning.
- The battle-end ALLY SUMMARY prints the player's honor and the total shared.

===============================================================================
VALIDATION
===============================================================================
Validated in game on 2026-11-07 with hf_ally v@ALLY_VERSION@:
- config cleanup and pre-combat UI;
- Russian/English radio line switch;
- honor sharing: logs reported honor 512.9 (allies 256.5) with one ally and
  honor 1166.3 (allies 1166.3) with two allies; the post-battle save contained
  an ally m_xp=583.167, matching half of the two-ally award.

hf_ai and QoL are unchanged; their validation status is unchanged. No automated
regression tests are included.

===============================================================================
CONFIG COMPATIBILITY
===============================================================================
Updating from v0.14.22 or any later development build: replace
Modloader/config/ally.json with the file in this ZIP. The key order is grouped,
and retired keys are ignored if left in an old file.

The important value changes: enabled is the only master switch; spawn_ally,
ally_toggle, ally_reticles, max_enemy_ships, plane_bombers, marker_seam,
marker_delay_ms and the retired logging keys are gone; ally_damage_color is
HEX; ally_reticle_scale 0 hides the ally boxes; ally_honor_share defaults to
0.5. hf_ai.json is unchanged since hf_ai v0.10.11.

If you come from v0.14.13 or older, replace Modloader/config/hf_ai.json too
(or delete it so the mod generates a fresh preset).

===============================================================================
INSTALL
===============================================================================
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

===============================================================================
FILES AND VALIDATION
===============================================================================
- Modloader/mods/hf_ally_v@ALLY_VERSION@.dll, hf_ai_v@AI_VERSION@.dll, highfleet_qol.dll
- Modloader/config/ally.json, hf_ai.json, logging.json, qol.full.json
- Media/Snd/RADIO/radio_stall_01.wav: our Russian voice line, replacing that game file
- docs/: player-defaults, ally settings, retired settings/diagnostics, debug
  overlay, AI settings/tuning and QoL guide
- manifest.json: source commit, component versions, gameplay validation and
  per-file SHA256 hashes
- Built by GitHub Actions without diagnostic features. The hf_ally features
  listed above were validated in game by the maintainer; hf_ai and QoL are
  unchanged.

CREDITS
Built on logdot's Highfleet-Modloader, patchy-rs, highfleet-rs and highfleet-qol.
MIT licenses are included. Unofficial fan project; not affiliated with HighFleet's
creators or publisher. No base-game executable is included.
