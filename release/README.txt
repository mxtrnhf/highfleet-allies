HIGHFLEET ALLIES — PACK v@PACK_VERSION@
hf_ally @ALLY_VERSION@ + hf_ai @AI_VERSION@ + optional QoL build — Steam 1.163 only
Source: https://github.com/mxtrnhf/highfleet-allies
Research notes: https://github.com/mxtrnhf/hf_notes

===============================================================================
1.0.0 — FIRST STABLE RELEASE
===============================================================================
hf_ally v@ALLY_VERSION@ and hf_ai v@AI_VERSION@. The current build was validated
in game on 2026-11-08 and confirmed working. Back up your Saves folder first.

WHAT CHANGED SINCE PACK v0.15.0

hf_ally
- auto_retreat retired. flagship_leaves always applies: "handover" (default)
  promotes an ally and restores the retreat point; "retreat" extracts the squad
  and ends the battle as a retreat.
- enemy_all_at_once retired. It only skipped the stock 30 s wait before the
  first reinforcement wave. enemy_group_size still controls enemies active at
  once; the stock refill delay now always applies.
- Battle-side performance pass (v0.16.2): shared callback-local views for panel
  and HUD-marker discovery, streaming interceptor target selection, precombat
  cleanup on screen exit, and bounded debug work. No config changes.
- Warning cleanup: removed unused constants/helpers and fixed a function-cast
  warning. No behaviour change.
- Existing ally features remain: wingman deployment, ALLY and plane-loadout
  buttons, language-aware radio lines, ally panels and target boxes, ship
  switching, carrier loadouts, K-13 interceptors, optional ship bombers, fuel
  retreat, prize sparing, and ally_honor_share (default 0.5).

hf_ai
- Preventive flare curtain retired; reactive flare bursts, ordinary evasion,
  panic evasion, barrel rolls, per-missile reaction rolls and stray-missile
  evasion remain.
- Offensive early fire retired (aim_fire_probability, aim_fire_range and the
  hidden aim_fire_s). Native firing is untouched; aim dodging and dive
  avoidance remain.
- Default dive_dwell_s is now 0.3 s (was 0.5). Dive avoidance rolls sooner.
- Universal aim dodging, dive avoidance, reactive missile defence, carrier
  launch groups and Zenith burst dumping remain.

QoL
- The bundled QoL build is unchanged from pack v0.15.0.

===============================================================================
CONFIG COMPATIBILITY
===============================================================================
Replace both config files with the 1.0.0 presets:

- Modloader/config/ally.json
- Modloader/config/hf_ai.json

Old keys are accepted but ignored: auto_retreat, enemy_all_at_once, spawn_ally,
ally_toggle, ally_reticles, max_enemy_ships, plane_bombers, marker_seam,
marker_delay_ms, the retired logging keys, curtain_* / launch_flares_*, and
aim_fire_*.

If you keep an old hf_ai.json, its explicit dive_dwell_s: 0.5 still wins; use
the new preset to get 0.3.

===============================================================================
VALIDATION
===============================================================================
- In-game validated on 2026-11-08 with the current 1.0.0 build: the mod runs
  and plays correctly.
- Earlier ally validation on 2026-11-07 covered config cleanup, pre-combat UI,
  Russian/English radio selection and battle honor/XP sharing.
- No automated regression tests are included. Cross-build success is not a
  substitute for in-game validation.

===============================================================================
INSTALL
===============================================================================
1. Back up Saves.
2. Read Allies_Manual.pdf.
3. Install logdot's Modloader v4:
   https://github.com/logdot/Highfleet-Modloader
4. Extract this ZIP directly into the folder containing Highfleet.exe. Merge
   Modloader/ and Media/, and remove previous hf_ally_v*.dll / hf_ai_v*.dll
   files from Modloader/mods/ so only ONE of each is present.
5. Use the included Modloader/config/ally.json and hf_ai.json as the new
   presets, or merge them into your existing custom profile. Do not keep the
   retired keys.
6. Keep the QoL flare crash fix enabled. The optional highfleet_qol.dll is our
   logdot QoL main + manual editor elevation and firing-arc rules build.

===============================================================================
FILES
===============================================================================
- Allies_Manual.pdf: player manual (14 pages)
- Modloader/mods/hf_ally_v@ALLY_VERSION@.dll, hf_ai_v@AI_VERSION@.dll, highfleet_qol.dll
- Modloader/config/ally.json, hf_ai.json, logging.json, qol.full.json
- Media/Snd/RADIO/radio_stall_01.wav: our Russian voice line, replacing that game file
- docs/: player defaults, ally settings, retired settings/diagnostics, debug
  overlay, AI settings/tuning and QoL guide
- manifest.json: source commit, component versions, gameplay validation and
  per-file SHA256 hashes
- Built by GitHub Actions without diagnostic features.

CREDITS
Built on logdot's Highfleet-Modloader, patchy-rs, highfleet-rs and highfleet-qol.
MIT licenses are included. Unofficial fan project; not affiliated with HighFleet's
creators or publisher. No base-game executable is included.
