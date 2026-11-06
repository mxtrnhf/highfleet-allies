# HighFleet Allies v@PACK_VERSION@

For **HighFleet Steam 1.163**.

> ## Config cleanup
> This is an ally-side config cleanup. `logging.debug_log` is now the runtime debug switch and is off by default. `debug_overlay` moved under `logging`; `max_lines`, `log_precombat`, `probe_lines`, `log_colors` and `dead_node` are retired. `hf_ai` and QoL are unchanged.
>
> ## Config compatibility
> **Updating from v0.14.18:** replace the `logging` block in `Modloader/config/ally.json`. `logging.debug_overlay` replaces the top-level `debug_overlay` key (the old key still loads). Retired logging keys are ignored. The `hf_ai.json` preset is unchanged since hf_ai v0.10.11.
>
> **Updating from v0.14.16 or older:** no separate `hf_ai.json` change is needed unless you come from v0.14.13 or older, where the hf_ai preset still has old dive/flare values: replace `Modloader/config/hf_ai.json` with the one in this pack (or delete it so the mod generates a fresh preset).

## Changes since v0.14.18

### `hf_ally v@ALLY_VERSION@` — config cleanup
- `logging.debug_log` is the runtime on/off switch for debug mode and defaults to `false`; the legacy `megalog` cargo feature no longer forces it on.
- Debug mode still provides the megalog, watchdog, crash dump, pre-combat GUI lists, per-second probes and the lifted line budget.
- `logging.max_lines`, `logging.log_precombat` and `logging.probe_lines` are retired; the internal budget is 600 lines normally and lifted in debug mode.
- `logging.log_colors` and `logging.dead_node` are retired; their sources are kept under `ally/src/retired/` and documented in `docs/retired-diagnostics.md`.
- `logging.gui_inspect` and `logging.log_creatures` remain available, both default false.
- `debug_overlay` moved to `logging.debug_overlay` (default false); the old top-level key still loads.

### Updated ally config
- The pack's `Modloader/config/ally.json` carries the new `logging` block. Merge it into your existing config; an old config still loads.
- `Modloader/config/hf_ai.json` is unchanged.

### Validation
- Cross-build only (`cargo xwin build --release --target x86_64-pc-windows-msvc`, and with the legacy `megalog` feature); no in-game validation was performed for this cleanup.
- No automated regression tests were added.

## Pack contents

`hf_ally v@ALLY_VERSION@`, `hf_ai v@AI_VERSION@`, and the bundled QoL build.

## Installation

1. Back up your `Saves` folder.
2. Extract the pack into the folder containing `Highfleet.exe`.
3. Remove any older `hf_ai_v*.dll` and `hf_ally_v*.dll` files from `Modloader/mods/` so only one of each DLL is present.
4. Merge the new `Modloader/config/ally.json` key into your existing config if you keep a custom profile.
