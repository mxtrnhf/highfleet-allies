# HighFleet Allies v@PACK_VERSION@

For **HighFleet Steam 1.163**.

> ## Config grouping
> This is an ally-side config organization update. The preset is grouped by feature; values and behaviour are unchanged. Enemy-wave docs now distinguish total enemy list size from the number active at once. `hf_ai` and QoL are unchanged.
>
> ## Config compatibility
> **Updating from v0.14.20:** no schema change. Merge the grouped preset if you want the same layout. `max_enemy_ships` is the total enemy list size; `enemy_group_size` is the number of enemy slots active at once. The `hf_ai.json` preset is unchanged since hf_ai v0.10.11.
>
> **Updating from v0.14.19 or older:** also apply the earlier cleanups: `plane_bombers`, `marker_seam` and `marker_delay_ms` are retired; `ally_damage_color` is HEX; `logging.debug_log` defaults to `false`, `logging.debug_overlay` replaces the top-level key, and the retired logging keys are ignored. If you come from v0.14.13 or older, replace `Modloader/config/hf_ai.json` too.

## Changes since v0.14.20

### `hf_ally v@ALLY_VERSION@` — config grouping
- The player preset and generated default config are grouped by feature: deployment/retreat, ally combat/UI, aircraft, enemies, markers, logging, and intercept.
- No values or gameplay behaviour changed.
- Corrected the enemy-wave explanation: battle setup places the stock three-ship first group. `max_enemy_ships` is the total enemy list size; `enemy_group_size` is the active-slot target for the refill function. After the 30 s timer the live group is topped up every tick as slots free up, not a new three-ship wave every 30 s. `enemy_all_at_once` only sets that timer to 0.

### Updated ally config
- The pack's `Modloader/config/ally.json` is grouped by feature. Existing values are retained.
- `Modloader/config/hf_ai.json` is unchanged.

### Validation
- Cross-build only (`cargo xwin build --release --target x86_64-pc-windows-msvc`); no in-game validation was needed for config ordering.
- No automated regression tests were added.

## Pack contents

`hf_ally v@ALLY_VERSION@`, `hf_ai v@AI_VERSION@`, and the bundled QoL build.

## Installation

1. Back up your `Saves` folder.
2. Extract the pack into the folder containing `Highfleet.exe`.
3. Remove any older `hf_ai_v*.dll` and `hf_ally_v*.dll` files from `Modloader/mods/` so only one of each DLL is present.
4. Merge the new `Modloader/config/ally.json` key into your existing config if you keep a custom profile.
