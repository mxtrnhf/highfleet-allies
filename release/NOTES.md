# HighFleet Allies v@PACK_VERSION@

For **HighFleet Steam 1.163**.

> ## Enemy config cleanup
> This removes the misleading `max_enemy_ships` setting. It patched the strategic bomber scene, not normal tactical combat; vanilla tactical battles have no total enemy cap. Use `enemy_group_size` to control how many enemies are active at once. `hf_ai` and QoL are unchanged.
>
> ## Config compatibility
> **Updating from v0.14.21:** remove or ignore `max_enemy_ships`; it is retired. `enemy_group_size` controls both the initial enemy group and refills after the 30 s timer. `enemy_all_at_once` only sets that timer to 0. The `hf_ai.json` preset is unchanged since hf_ai v0.10.11.
>
> **Updating from v0.14.20 or older:** also apply the earlier cleanups: the ally preset is grouped by feature; `plane_bombers`, `marker_seam` and `marker_delay_ms` are retired; `ally_damage_color` is HEX; `logging.debug_log` defaults to `false`, `logging.debug_overlay` replaces the top-level key, and the retired logging keys are ignored. If you come from v0.14.13 or older, replace `Modloader/config/hf_ai.json` too.

## Changes since v0.14.21

### `hf_ally v@ALLY_VERSION@` — retire `max_enemy_ships`
- Removed the key and its patch. The patched instruction was in `SCR_BOMBERS`, the strategic bomber scene setup, not `SCR_COMBAT`.
- Normal tactical combat fills its enemy list from the engagement through `Precombat_SplitFleetIntoReserveLists` with no total cap, so the setting did nothing in normal battles.
- `enemy_group_size` is the real control for enemies active at once: it is used for the initial group and for refills after the 30 s timer. The weight budget is `3 × group_size`.
- `enemy_all_at_once` only sets the 30 s refill timer to 0.

### Updated ally config
- The pack's `Modloader/config/ally.json` drops `max_enemy_ships`; the grouped layout is unchanged.
- `Modloader/config/hf_ai.json` is unchanged.

### Validation
- Cross-build only (`cargo xwin build --release --target x86_64-pc-windows-msvc`); no in-game validation was performed for this config removal.
- No automated regression tests were added.

## Pack contents

`hf_ally v@ALLY_VERSION@`, `hf_ai v@AI_VERSION@`, and the bundled QoL build.

## Installation

1. Back up your `Saves` folder.
2. Extract the pack into the folder containing `Highfleet.exe`.
3. Remove any older `hf_ai_v*.dll` and `hf_ally_v*.dll` files from `Modloader/mods/` so only one of each DLL is present.
4. Merge the new `Modloader/config/ally.json` key into your existing config if you keep a custom profile.
