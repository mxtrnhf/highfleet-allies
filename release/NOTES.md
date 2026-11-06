# HighFleet Allies v@PACK_VERSION@

For **HighFleet Steam 1.163**.

> ## Ally config cleanup
> This pack cleans up the ally configuration and removes retired settings. `hf_ai` and QoL are unchanged. The pack's `Modloader/config/ally.json` is the current grouped preset.
>
> ## Config compatibility
> **Updating from v0.14.18 or older:** replace `Modloader/config/ally.json` from this pack (or merge the current keys). Retired keys are ignored if left in an old file. If you come from v0.14.13 or older, replace `Modloader/config/hf_ai.json` too. The `hf_ai.json` preset is unchanged since hf_ai v0.10.11.

## Changes since v0.14.18

### `hf_ally v@ALLY_VERSION@` — config cleanup
- `logging.debug_log` is the runtime debug switch and defaults to `false`. Debug mode still provides the megalog, watchdog, crash dump, pre-combat GUI lists, per-second probes and the lifted line budget. The legacy `megalog` cargo feature no longer forces it on.
- `debug_overlay` moved to `logging.debug_overlay` (default false).
- `max_enemy_ships` is retired. Its patch was inside `SCR_BOMBERS`, the strategic bomber scene setup, not `SCR_COMBAT`; normal tactical combat fills its enemy list from the engagement with no total cap. Use `enemy_group_size` for enemies active at once.
- `plane_bombers` is retired. It was the unsafe global patch that could send planes into the diving run before they cleared the carrier. Tactical bombing remains available through `plane_loadouts` plus `plane_attack_delay`.
- `marker_seam` and `marker_delay_ms` are retired. Labels always use the fixed B23 seam; the roller-cover check already hides them while the doors slide.
- `ally_damage_color` is written as HEX (`"#00E4FF"`); numeric values from older configs still load.
- The remaining keys are grouped by feature in the preset and generated defaults.

### Enemy groups
- `enemy_group_size` controls the enemy slots active at once: the initial group and refills after the 30 s timer. The weight budget is `3 × group_size`, so expensive ships can make the actual group smaller.
- `enemy_all_at_once` only sets the 30 s refill timer to 0. It does not change the initial group size.

### Updated ally config
- The pack's `Modloader/config/ally.json` is the current grouped preset without retired keys.
- `Modloader/config/hf_ai.json` is unchanged.

### Validation
- Cross-build only (`cargo xwin build --release --target x86_64-pc-windows-msvc`); no in-game validation was performed for this config cleanup.
- No automated regression tests were added.

## Pack contents

`hf_ally v@ALLY_VERSION@`, `hf_ai v@AI_VERSION@`, and the bundled QoL build.

## Installation

1. Back up your `Saves` folder.
2. Extract the pack into the folder containing `Highfleet.exe`.
3. Remove any older `hf_ai_v*.dll` and `hf_ally_v*.dll` files from `Modloader/mods/` so only one of each DLL is present.
4. Merge the new `Modloader/config/ally.json` key into your existing config if you keep a custom profile.
