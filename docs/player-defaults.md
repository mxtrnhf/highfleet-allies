# Player defaults — release pack v0.14.15

> **IMPORTANT — UPDATE YOUR `Modloader/config/hf_ai.json`!**
> When upgrading to pack v0.14.15 (`hf_ai v0.10.11`), replace your old `Modloader/config/hf_ai.json` with the new preset from the pack (or delete your old `hf_ai.json` so `hf_ai` generates a fresh copy).

The tracked player default profiles for `hf_ally v0.14.15` and `hf_ai v0.10.11` live in:

- [ally/config/ally.json](../ally/config/ally.json)
- [ai/config/hf_ai.json](../ai/config/hf_ai.json)

The release pack copies these files unchanged. Compiled defaults use the same
values: a missing file is generated with this profile, and missing keys or whole
blocks in a partial config inherit it. Explicit values in an existing config
continue to override the defaults. Invalid JSON logs an error and uses the profile.
The presets are not vanilla settings; use the feature-off tables in the config
guides to disable specific behavior.

## Profile highlights

Allies (`hf_ally v0.14.15`): up to four wingmen, 1.5 s arrival delay, eight enemies,
vanilla three-ship waves, K-13/loadout buttons on, ally ship panels and ally target
boxes on (`ally_reticle_scale: 0.6`), ship switching on, ally ship bombers on,
bomb/rocket and AA attack delays of 1 s, and fuel retreat at 30%. The Sprint
exception is `MDL_MISSILE_03`. Battle `debug_overlay` defaults to `false`. Runtime
`logging.debug_log` is `true`; set it `false` for quieter play.

AI (`hf_ai v0.10.11`): trimmed 38-key config. Flares, allied reactions, universal
aim dodging (`aim_chance: 0.75`, `aim_boost: 2.0`), offensive early fire
(`aim_fire_probability: 0.5`, `aim_fire_range: 1.6`, with friendly-fire corridor
check), and dive avoidance (`dive_chance: 0.8`, `dive_dwell_s: 0.5`,
`dive_boost: 2.0`) are on. Top-level `twr_cutoff` is `2.0`. Carrier groups launch
up to three planes at `0.5` s intervals. Missile-dump chance is `0.4` with
`0.6..1.5` s spacing (for both enemies and allies). Plain evasion strength is
`0.5`, roll chance `0.001`, panic-evasion chance `0.1` and panic duration `5` s.
Preventive curtain retreat range (`launch_flares_evade_range`) is `400.0`.
`logging.log_panic` is off. Internal math/timing keys are omitted from the preset
and documented under Retired settings in [hf-ai-config-reference.md](hf-ai-config-reference.md).

## Maintaining the reference

Update the tracked preset and corresponding Rust defaults together.

Build the three DLLs with `cargo xwin build --release --locked --no-default-features
--target x86_64-pc-windows-msvc` in `ally/`, `ai/` and `qol/`. Commit the source and
presets, then run `python3 tools/package_release.py --version 0.14.15 --output
../release_packs`. The packager checks a clean tracked source tree and includes a
manifest with the source commit, versions and per-file SHA256 hashes.
