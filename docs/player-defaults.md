# Player defaults — release pack v0.14.2

The player's installed configs were captured on 2026-10-30 as the reference
profile for hf_ally 0.14.2 and hf_ai 0.10.7:

- [ally/config/ally.json](../ally/config/ally.json)
- [ai/config/hf_ai.json](../ai/config/hf_ai.json)

The release pack copies these files unchanged. Compiled defaults use the same
values: a missing file is generated with this profile, and missing keys or whole
blocks in a partial config inherit it. Explicit values in an existing config
continue to override the defaults. Invalid JSON logs an error and uses the profile.
The presets are not vanilla settings; use the feature-off tables in the config
guides to disable specific behavior.

## Profile highlights

Allies: two wingmen, 1.5 s arrival delay, eight enemies, vanilla three-ship waves,
K-13/loadout buttons on, bomb/rocket and AA attack delays of 1 s, and fuel retreat
at 30%. The Sprint exception is `MDL_MISSILE_03`. Runtime `debug_log` is **true**, as
in the captured player config. That enables detailed logs, the watchdog and crash
dumps, and can force diagnostic probe/pre-combat output despite their individual
keys being false. Set it false for quieter play. Telemetry is false, and player
release DLLs are compiled without the telemetry or megalog features.

AI: flares, allied reactions, aim and dive dodges are on; the top-level TWR cutoff
is 2.0. Carrier groups launch up to three planes at 0.5 s intervals. Missile-dump
chance is 0.4 with 0.6..1.5 s spacing. Plain evasion strength is 0.5, roll chance
0.001, panic-evasion chance 0.1 and panic duration 5 s. Early-fire chance is 0.5.
Steering, scan and panic logs are off. Optional band/circling/close-retreat
positioning and its settings remain removed.

The ally preset retains the player's numeric colour `58623`, equivalent to
`"#00E4FF"`. The typed config normalizes this to HEX when generating a new file;
the colour is unchanged. JSON key order is irrelevant.

## Maintaining the reference

Update the tracked preset and corresponding Rust defaults together. Regression
tests compare the typed preset with `Config::default()` and an empty `{}`. Run
`python3 tools/check_player_defaults.py --offline` for host-only validation; this
runs Rust tests on configuration/math code and never loads a mod DLL or the game.

Build the three DLLs with `cargo xwin build --release --locked --no-default-features
--target x86_64-pc-windows-msvc` in `ally/`, `ai/` and `qol/`. Commit the source and
presets, then run `python3 tools/package_release.py --version 0.14.2 --output
../release_packs`. The packager checks a clean tracked source tree and includes a
manifest with the source commit, versions and per-file SHA256 hashes. It does not
publish a release or modify the game installation.

The optional QoL build and its `qol.full.json` reference are unchanged in scope.
The pack does not ship an active `qol.json`, so it does not replace existing QoL
settings. The voice asset and logging/QoL reference files live under `release/`.
Old local snapshots under the workspace's `config_references/` remain untouched.
