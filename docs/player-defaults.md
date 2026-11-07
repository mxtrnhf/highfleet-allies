# Player defaults — hf_ally v0.16.0

> **Config compatibility.** Updating from v0.14.22 or any later development build: replace `Modloader/config/ally.json` with the v0.16.0 preset. `enabled` is the only master switch; `auto_retreat` is retired and `flagship_leaves` always applies; `spawn_ally`, `ally_toggle`, `ally_reticles`, `max_enemy_ships`, `plane_bombers`, `marker_seam`, `marker_delay_ms` and the retired logging keys are ignored. `ally_reticle_scale: 0` hides the ally target boxes. New `ally_honor_share` (default `0.5`) shares the player's battle honor with deployed allies. Ally radio lines follow the game language (`Media/Snd/RADIO` in Russian, `Media/Snd/RADIO_ENG` in English). `hf_ai.json` values are unchanged since `hf_ai v0.10.11`; `hf_ai v0.10.18` retires the preventive flare curtain and removes its `curtain_*` / `launch_flares_*` keys.
>
> **From v0.14.13 or older:** also replace `Modloader/config/hf_ai.json` (the v0.10.11 preset trims it to 49 leaf settings). From v0.14.21 or older, the earlier config cleanups are included in this preset: grouped by feature, HEX `ally_damage_color`, `logging.debug_log` default false, `logging.debug_overlay` under `logging`, and the retired logging keys.

The tracked player default profiles for `hf_ally v0.16.0` and `hf_ai v0.10.18` live in:

- [ally/config/ally.json](../ally/config/ally.json)
- [ai/config/hf_ai.json](../ai/config/hf_ai.json)

The v0.16.0 development preset uses the same `hf_ai.json` values as v0.14.15
and later, with the v0.10.18 curtain removal; `ally.json` drops `auto_retreat`, `spawn_ally`, `ally_toggle`, `ally_reticles` and the
misleading `max_enemy_ships` key, and adds `ally_honor_share: 0.5`. See the
[ally changelog](../ally/CHANGELOG.md) and [ai changelog](../ai/CHANGELOG.md).

The release pack copies these files unchanged. Compiled defaults use the same
values: a missing file is generated with this profile, and missing keys or whole
blocks in a partial config inherit it. Explicit values in an existing config
continue to override the defaults. Invalid JSON logs an error and uses the profile.
The presets are not vanilla settings; use the feature-off tables in the config
guides to disable specific behavior.

## Profile highlights

Allies (`hf_ally v0.16.0`): up to two wingmen, 1.5 s arrival delay,
three enemy slots active at once, K-13/loadout buttons on, ally ship panels and ally target
boxes on (`ally_reticle_scale: 0.8`), ship switching on, ally ship bombers off,
`ally_zenith_autofire` on (vanilla autonomous launches), bomb/rocket and AA attack
delays of 1 s, fuel retreat at 30%, and battle honor sharing at 50%
(`ally_honor_share: 0.5`). The Sprint exception is `MDL_MISSILE_03`.
`ally_damage_color` is `"#00E4FF"`. `logging.debug_overlay` defaults to `false`.
`logging.debug_log` is the runtime debug switch and defaults to `false`; set it
`true` only for a tester log.

AI (preset introduced in `hf_ai v0.10.11`): 48 gameplay settings plus `logging.log_panic`
(49 total leaf settings). Flares, allied reactions, universal
aim dodging (`aim_chance: 0.75`, `aim_boost: 2.0`), offensive early fire
(`aim_fire_probability: 0.5`, `aim_fire_range: 1.6`; v0.10.12 retains native
firing checks rather than forcing a fire byte), and dive avoidance (`dive_chance: 0.8`, `dive_dwell_s: 0.5`,
`dive_boost: 2.0`) are on. Top-level `twr_cutoff` is `2.0`. Carrier groups launch
up to three planes at `0.5` s intervals. Missile-dump chance is `0.4` with
`0.6..1.5` s spacing (for both enemies and allies). Plain evasion strength is
`0.5`, roll chance `0.001`, panic-evasion chance `0.1` and panic duration `5` s.
The preventive flare curtain is retired in v0.10.18. `logging.log_panic` is off. Internal math/timing keys are omitted from the preset
and documented under Retired settings in [hf-ai-config-reference.md](hf-ai-config-reference.md).

## Maintaining the reference

Update the tracked preset and corresponding Rust defaults together.

Build the three DLLs with `cargo xwin build --release --locked --no-default-features
--target x86_64-pc-windows-msvc` in `ally/`, `ai/` and `qol/`. Commit the source and
presets, then run `python3 tools/package_release.py --version 0.16.0 --output
../release_packs`. The packager checks a clean tracked source tree and includes a
manifest with the source commit, versions and per-file SHA256 hashes.
