HIGHFLEET ALLIES — PACK v@PACK_VERSION@
hf_ally @ALLY_VERSION@ + hf_ai @AI_VERSION@ + optional QoL build — Steam 1.163 only
Source: https://github.com/mxtrnhf/highfleet-allies
Research notes: https://github.com/mxtrnhf/hf_notes

WHAT CHANGED SINCE PACK v0.14.0
- Both mods now use the captured player configs as compiled defaults and release
  presets. Missing keys inherit the same values; existing explicit values remain.
- hf_ai: configurable TWR cutoff (player default 2.0), tactical carrier groups of
  3 aircraft at 0.5 s spacing, and consistent tactics.allies naming. The old
  tactics.apply_to_allies spelling is still accepted as an alias.
- The optional circling/preferred-distance/close-retreat routine is removed.
  Its settings, strafe_enabled and the old acceleration thresholds are deprecated
  and ignored. Aim/dive dodges, missile evasion and shared safety checks remain.
- hf_ally uses pinned highfleet-rs aircraft/loadout and GUI layouts, retaining
  game-native construction and ownership operations.

INSTALL
1. Back up Saves and your current ally.json/hf_ai.json before extracting. These
   supplied configs replace them with the player defaults.
2. Install logdot's Modloader v4:
   https://github.com/logdot/Highfleet-Modloader
3. Extract this ZIP directly into the folder containing Highfleet.exe. Merge
   Modloader/ and Media/. Keep only ONE hf_ally DLL and ONE hf_ai DLL in
   Modloader/mods; remove previous versions before loading the new ones.
4. Keep the QoL flare crash fix enabled. The optional highfleet_qol.dll is our
   existing logdot QoL main + manual editor elevation build, not a requirement
   to replace an already compatible QoL installation. It replaces an existing
   DLL of that name if copied, so back yours up. Once upstream releases those
   changes, prefer the upstream release.

PLAYER DEFAULTS
Modloader/config/ally.json and hf_ai.json are exact captured player presets.
They also define the compiled missing-key defaults; use enabled=false to turn
an entire mod off. See docs/player-defaults.md and the complete config guides.

Allies: max_allies=2, max_enemy_ships=8, plane_loadouts/plane_k13 on,
plane_attack_delay=1.0, plane_aa_delay=1.0, ally_fuel_retreat_percent=30.0.
Runtime debug_log=true is retained from the player's config. This enables the
megalog, watchdog and crash dumps; set it false for quieter play. Telemetry is
false and these DLLs are built without telemetry/megalog cargo features.

AI: twr_cutoff=2.0, tactics/flares/allied reactions on, missile dump chance 0.4
with 0.6..1.5 s delay, plane_group_per_launch=3 and delay=0.5 s. Evasion strength
0.5; barrel-roll chance 0.001; panic-evasion chance 0.1 and duration 5 s. Aim early
fire chance 0.5. Steering/scan/panic logs are off. Restart after editing settings.

Modloader/config/qol.full.json is a reference, NOT an active config: copy sections
into qol.json as desired. No active qol.json is supplied or overwritten.

FILES AND VALIDATION
- Modloader/mods/hf_ally_v@ALLY_VERSION@.dll, hf_ai_v@AI_VERSION@.dll, highfleet_qol.dll
- Modloader/config/ally.json, hf_ai.json, logging.json, qol.full.json
- Media/Snd/RADIO/radio_stall_01.wav: our voice line, replacing that game file
- docs/: player-defaults, ally settings, AI settings/tuning and QoL guide
- manifest.json: source commit, component versions and per-file SHA256 hashes

Windows release builds and test compilation are checked. Host configuration and
math tests are run. The agent does not execute DLLs or the game; gameplay testing
is left to the player. For bug reports, send the newest Modloader/logs/Highfleet_*.log
and reproduction steps. Crash dumps may contain paths/campaign data; send them
only when needed for diagnosis.

CREDITS
Built on logdot's Highfleet-Modloader, patchy-rs, highfleet-rs and highfleet-qol.
The optional QoL DLL includes upstream refuel/speed work and our editor elevation;
see https://github.com/logdot/highfleet-qol/pull/20 and the source's qol/ directory.
MIT licenses are included. Unofficial fan project; not affiliated with HighFleet's
creators or publisher. No base-game executable is included.
