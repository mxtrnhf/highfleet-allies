# HighFleet Allies v@PACK_VERSION@

For **HighFleet Steam 1.163**.

> ## Config cleanup
> This is an ally-side config cleanup. `plane_bombers`, `marker_seam` and `marker_delay_ms` are retired. `ally_damage_color` is written as HEX. `hf_ai` and QoL are unchanged.
>
> ## Config compatibility
> **Updating from v0.14.19:** remove or ignore `plane_bombers`, `marker_seam` and `marker_delay_ms`; they are ignored. The preset writes `ally_damage_color` as `"#00E4FF"`; numeric values from older configs still load. The `hf_ai.json` preset is unchanged since hf_ai v0.10.11.
>
> **Updating from v0.14.18 or older:** also replace the `logging` block. `logging.debug_log` defaults to `false`, `logging.debug_overlay` replaces the top-level key, and the retired logging keys are ignored. If you come from v0.14.13 or older, replace `Modloader/config/hf_ai.json` too.

## Changes since v0.14.19

### `hf_ally v@ALLY_VERSION@` — config cleanup
- `plane_bombers` is retired. It was the unsafe global patch that could send planes into the bombing dive before they cleared the carrier. Tactical bombing remains available through `plane_loadouts` plus `plane_attack_delay`.
- `marker_seam` and `marker_delay_ms` are retired. The PLAYER and plane-loadout labels now always use the fixed B23 seam; the roller-cover check already hides them while the doors slide.
- The preset writes `ally_damage_color` as `"#00E4FF"`. The config parser still accepts a plain numeric colour from older files.

### Updated ally config
- The pack's `Modloader/config/ally.json` drops the three retired keys and uses the HEX colour. Merge it into your existing config; an old config still loads.
- `Modloader/config/hf_ai.json` is unchanged.

### Validation
- Cross-build only (`cargo xwin build --release --target x86_64-pc-windows-msvc`); no in-game validation was performed for this cleanup.
- No automated regression tests were added.

## Pack contents

`hf_ally v@ALLY_VERSION@`, `hf_ai v@AI_VERSION@`, and the bundled QoL build.

## Installation

1. Back up your `Saves` folder.
2. Extract the pack into the folder containing `Highfleet.exe`.
3. Remove any older `hf_ai_v*.dll` and `hf_ally_v*.dll` files from `Modloader/mods/` so only one of each DLL is present.
4. Merge the new `Modloader/config/ally.json` key into your existing config if you keep a custom profile.
