HIGHFLEET ALLIES — PACK v@PACK_VERSION@
hf_ally @ALLY_VERSION@ + hf_ai @AI_VERSION@ + optional QoL build — Steam 1.163 only
Source: https://github.com/mxtrnhf/highfleet-allies
Research notes: https://github.com/mxtrnhf/hf_notes

WHAT CHANGED SINCE PACK v0.14.2
- Adjusted default launch_flares_evade_range to 700.0 to stop AI from overusing
  the afterburner.
- Fixed stale diagnostic probes after restarting a battle.
- Moved the stock aircraft AI fix and null-fleet/vector crash guards into bundled
  QoL. fix_plane_ai_crash and fix_merge_crash now belong in qol.json (default true).
  Update both DLLs together. Upstream QoL does not include these two additions.
  The fleet guards are defensive, not a confirmed fix for every merge crash.
- Removed ship/battle, steering and obstacle CSV capture and replay-analysis tools.
- Kept megalog/crash reports and text diagnostics; settings now live under logging.
  Old logging-key locations remain readable. Dead-node text logging is optional.
- Bundled QoL now supports elevation_rules: vanilla (default), modern, classic,
  classic_kaz. Vanilla/modern preserve 1.163 firing-arc rules; classic restores
  older gun/KAZ rules; classic_kaz additionally exempts trusses from KAZ scans.
  This is independent of manual editor elevation. See NOTES.md and docs/qol-guide.md.

INSTALL
1. Back up Saves and your current ally.json/hf_ai.json before extracting. These
   supplied configs replace them with the player defaults.
2. Install logdot's Modloader v4:
   https://github.com/logdot/Highfleet-Modloader
3. Extract this ZIP directly into the folder containing Highfleet.exe. Merge
   Modloader/ and Media/. Keep only ONE hf_ally DLL and ONE hf_ai DLL in
   Modloader/mods; remove previous versions before loading the new ones.
4. Keep the QoL flare crash fix enabled. The optional highfleet_qol.dll is our
    logdot QoL main + manual editor elevation and firing-arc rules build, not a requirement
   to replace an already compatible QoL installation. It replaces an existing
   DLL of that name if copied, so back yours up. Once upstream releases those
   changes, prefer the upstream release.

PLAYER DEFAULTS
Modloader/config/ally.json and hf_ai.json are exact captured player presets.
They also define the compiled missing-key defaults; use enabled=false to turn
an entire mod off. See docs/player-defaults.md and the complete config guides.

Allies: max_allies=2, max_enemy_ships=8, plane_loadouts/plane_k13 on,
plane_attack_delay=1.0, plane_aa_delay=1.0, ally_fuel_retreat_percent=30.0.
Runtime logging.debug_log=true is retained from the player's config. This enables
the megalog, watchdog and crash dumps; set it false for quieter play. Logging
options live under "logging" in ally.json and hf_ai.json. These DLLs are built
without the forced megalog cargo feature. Ship/battle CSV capture is removed.

AI: twr_cutoff=2.0, tactics/flares/allied reactions on, missile dump chance 0.4
with 0.6..1.5 s delay, plane_group_per_launch=3 and delay=0.5 s. Evasion strength
0.5; barrel-roll chance 0.001; panic-evasion chance 0.1 and duration 5 s. Aim early
fire chance 0.5. CSV capture is removed; panic text logging is off. Restart after editing settings.

Modloader/config/qol.full.json is a reference, NOT an active config: copy sections
into qol.json as desired. No active qol.json is supplied or overwritten.

FILES AND VALIDATION
- Modloader/mods/hf_ally_v@ALLY_VERSION@.dll, hf_ai_v@AI_VERSION@.dll, highfleet_qol.dll
- Modloader/config/ally.json, hf_ai.json, logging.json, qol.full.json
- Media/Snd/RADIO/radio_stall_01.wav: our voice line, replacing that game file
- docs/: player-defaults, ally settings, AI settings/tuning and QoL guide
- manifest.json: source commit, component versions and per-file SHA256 hashes

Windows release cross-builds are checked. Automated tests have been removed.
The agent does not execute DLLs or the game; gameplay testing
is left to the player. For bug reports, send the newest Modloader/logs/Highfleet_*.log
and reproduction steps. Crash dumps may contain paths/campaign data; send them
only when needed for diagnosis.

CREDITS
Built on logdot's Highfleet-Modloader, patchy-rs, highfleet-rs and highfleet-qol.
The optional QoL DLL includes upstream refuel/speed work, editor elevation and firing-arc rules;
see https://github.com/logdot/highfleet-qol/pull/20 and the source's qol/ directory.
MIT licenses are included. Unofficial fan project; not affiliated with HighFleet's
creators or publisher. No base-game executable is included.
