# HighFleet Allies v@PACK_VERSION@

For **HighFleet Steam 1.163**.

> ## What this pack is
> `hf_ally v@ALLY_VERSION@`, `hf_ai v@AI_VERSION@`, and the bundled QoL build. This is the first public pack since v0.14.22. It retires the redundant ally config switches, makes ally radio lines follow the game language, and adds battle honor/XP sharing for deployed allies. `hf_ai` and QoL are unchanged.
>
> **Validated in game** on 2026-11-07: config cleanup and pre-combat UI, Russian/English radio line selection, and ally honor/XP sharing. Back up your `Saves` folder before installing.

## Changes since v0.14.22

### `ally.json` changes
- `enabled` is the only master switch. `spawn_ally` and `ally_toggle` are retired and ignored. When enabled, the wingman deploys and the pre-combat ALLY/plane-loadout UI is installed; use the per-ship ALLY toggle to skip the wingman.
- `ally_reticles` is retired; `ally_reticle_scale` controls both size and visibility of the ally target boxes. `0` = off; positive values set the size; negative or non-finite values fall back to `0.8`.
- `max_enemy_ships` is retired. It patched `SCR_BOMBERS`, the strategic bomber scene, not normal tactical combat; normal combat has no total enemy cap. Use `enemy_group_size` for the number of enemies active at once.
- `plane_bombers`, `marker_seam`, `marker_delay_ms` and the retired logging keys (`logging.max_lines`, `logging.log_precombat`, `logging.probe_lines`, `logging.log_colors`, `logging.dead_node`) are ignored. `debug_overlay` lives under `logging`.
- `ally_damage_color` is written as HEX (`"#00E4FF"`); numeric values from older configs still load.
- The pre-combat ALLY and plane-loadout buttons use a per-button 120 ms debounce, so one physical click cannot toggle twice and a second button is not swallowed.
- The pack's `Modloader/config/ally.json` is the current grouped preset and contains no retired keys.

### Ally radio lines follow the game language
- Russian uses `Media/Snd/RADIO`; English uses `Media/Snd/RADIO_ENG`. This is the stock `_eng` `sound.res` selection and follows a live language change in the options.
- The pack keeps the existing custom Russian `Media/Snd/RADIO/radio_stall_01.wav`; English uses the stock English line.
- **Custom voice paths and custom audio for ally lines are planned for a future version; they are not in this pack.**
- The pre-combat `button_01` UI click is unchanged and is not localized.

### Battle honor / XP sharing
- New `ally_honor_share` (default `0.5`). The player keeps 100% of the honor/XP earned; every deployed ally receives 50% of the player's award as it is earned.
- `0` disables sharing; `1` gives allies the full award; values outside 0..=1 are clamped; non-finite values use `0.5`.
- The share uses the native battle honor field (`Creature+0x1a24`), which the game converts to each ship's persistent `m_xp`, stars and promotions after the battle. Late-joining allies share from deployment; a ship that leaves stops earning.
- The battle-end `ALLY SUMMARY` line prints the player's earned honor and the total shared with allies.

### Validation
- In-game validated on 2026-11-07 with `hf_ally v@ALLY_VERSION@`:
  - config cleanup and pre-combat UI;
  - Russian/English radio line switch;
  - honor sharing: the summary reported `honor 512.9 (allies 256.5)` with one ally and `honor 1166.3 (allies 1166.3)` with two allies; the post-battle save contained an ally `m_xp=583.167`, matching half of the two-ally award.
- `hf_ai` and QoL are unchanged from the previous pack; their validation status is unchanged.
- No automated regression tests are included. Cross-build success is not a substitute for in-game validation.

## Pack contents

`hf_ally v@ALLY_VERSION@`, `hf_ai v@AI_VERSION@`, and the bundled QoL build. The pack also keeps the existing custom Russian `Media/Snd/RADIO/radio_stall_01.wav`.

## Installation

1. Back up your `Saves` folder.
2. Extract the pack into the folder containing `Highfleet.exe`.
3. Remove any older `hf_ai_v*.dll`, `hf_ally_v*.dll` and duplicate `highfleet_qol.dll` from `Modloader/mods/` so only one of each is present.
4. Use the pack's `Modloader/config/ally.json` as the new preset, or merge it into your custom profile. Retired keys are ignored if left in an old file.
5. Keep the QoL flare crash fix enabled (default).

## Notes

- `Modloader/config/hf_ai.json` is unchanged since `hf_ai v0.10.11`.
- Steam 1.163 only.
- Research notes: https://github.com/mxtrnhf/hf_notes
