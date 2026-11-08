# HighFleet Allies v@PACK_VERSION@

For **HighFleet Steam 1.163**.

> ## 1.0.0 — first stable release
> `hf_ally v@ALLY_VERSION@`, `hf_ai v@AI_VERSION@`, and the bundled QoL build.
>
> **Validated in game** on 2026-11-08: the current build runs and plays correctly. Back up your `Saves` folder before installing.

## What changed since pack v0.15.0

### `hf_ally v1.0.0`

- **`auto_retreat` retired.** `flagship_leaves` always applies: `"handover"` (default) promotes an ally and restores the retreat point; `"retreat"` extracts the squad and ends the battle as a retreat.
- **`enemy_all_at_once` retired.** It only skipped the stock 30-second wait before the first reinforcement wave. `enemy_group_size` still controls how many enemies are active at once; the stock refill delay now always applies.
- **Ally ship bombers are enabled by default** (`ally_bombers: true`), and the default ally target-box scale is `0.6`. Both values mirror the maintainer's validated in-game configuration. Set `ally_bombers: false` to disable ship bombing.
- **Battle-side performance pass (v0.16.2).** Panel and HUD-marker discovery now shares one validated view per callback instead of walking the same native tables repeatedly. Interceptor target selection streams the active-body list instead of building an all-body snapshot every hunt. Precombat cleanup runs on screen exit rather than on every armed combat tick. Debug probes are bounded and logging-only diagnostics respect `logging.debug_log`. No config keys, dependencies or persistent native-pointer caches were added.
- **Warning cleanup.** Removed four unused constants/helpers and fixed a function-pointer cast warning. No behaviour change.
- Existing ally features remain: wingman deployment, pre-combat ALLY and plane-loadout buttons, language-aware radio lines, ally panels and target boxes, ship switching, carrier loadouts, K-13 interceptors, optional ally ship bombers, fuel retreat, prize sparing, and battle honor/XP sharing (`ally_honor_share`, default `0.5`).

### `hf_ai v1.0.0`

- **Preventive flare curtain retired.** The proactive pre-lock flare cloud and its `curtain_*` / `launch_flares_*` settings are gone. Reactive missile defence remains: flare bursts, ordinary evasion, panic evasion, barrel rolls, per-missile reaction rolls and stray-missile evasion.
- **Offensive early fire retired.** The group-0 range-permission hook and the `aim_fire_probability` / `aim_fire_range` / `aim_fire_s` settings are gone. Native firing is untouched; aim dodging and dive avoidance remain.
- **Default `dive_dwell_s` is now `0.3` s** (was `0.5`). Dive avoidance rolls sooner when a target stays in the vertical cone.
- Existing AI features remain: universal aim dodging, dive avoidance, reactive missile defence, carrier launch groups and Zenith burst dumping.

### QoL

The bundled QoL build is unchanged from pack v0.15.0.

## Config compatibility

Replace both config files with the 1.0.0 presets:

- `Modloader/config/ally.json`
- `Modloader/config/hf_ai.json`

Old keys are accepted but ignored: `auto_retreat`, `enemy_all_at_once`, `spawn_ally`, `ally_toggle`, `ally_reticles`, `max_enemy_ships`, `plane_bombers`, `marker_seam`, `marker_delay_ms`, the retired logging keys, `curtain_*` / `launch_flares_*`, and `aim_fire_*`.

If you keep an old `hf_ai.json`, its explicit `dive_dwell_s: 0.5` still wins; use the new preset to get `0.3`.

The 1.0.0 `ally.json` also enables `ally_bombers` and uses `ally_reticle_scale: 0.6`; older explicit values still win until you replace the file.

## Validation

- In-game validated on 2026-11-08 with the current 1.0.0 build: the mod runs and plays correctly.
- Earlier ally validation on 2026-11-07 covered the config cleanup, pre-combat UI, Russian/English radio selection and battle honor/XP sharing.
- No automated regression tests are included. Cross-build success is not a substitute for in-game validation.

## Pack contents

`hf_ally v@ALLY_VERSION@`, `hf_ai v@AI_VERSION@`, and the bundled QoL build. The pack also includes the player manual `Allies_Manual.pdf` (14 pages) and keeps the existing custom Russian `Media/Snd/RADIO/radio_stall_01.wav`.

## Installation

1. Back up your `Saves` folder.
2. Read `Allies_Manual.pdf`.
3. Extract the pack into the folder containing `Highfleet.exe`.
4. Remove any older `hf_ai_v*.dll`, `hf_ally_v*.dll` and duplicate `highfleet_qol.dll` from `Modloader/mods/` so only one of each is present.
5. Use the pack's `Modloader/config/ally.json` and `Modloader/config/hf_ai.json` as the new presets, or merge them into your custom profile. Retired keys are ignored if left in an old file.
6. Keep the QoL flare crash fix enabled (default).

## Notes

- Steam 1.163 only.
- Research notes: https://github.com/mxtrnhf/hf_notes
