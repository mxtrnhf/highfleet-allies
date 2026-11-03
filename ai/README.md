# hf_ai v0.10.12

> **IMPORTANT — UPDATE YOUR `Modloader/config/hf_ai.json`!**
> When upgrading from before `hf_ai v0.10.11`, replace your existing `Modloader/config/hf_ai.json` with the tracked preset (or delete it so the mod writes fresh defaults). `v0.10.11` trims the preset from 80 to 49 total leaf settings (48 gameplay settings plus `logging.log_panic`), reworks aim dodging and early fire across all ships, merges dive settings, and retires curtain sheltering. The preset is unchanged in v0.10.12; this follow-up is not a published release pack.

A tactical AI layer for **HighFleet Steam 1.163**, loaded as a standalone DLL by
logdot's Modloader v4. The stock AI still selects targets and computes movement;
hf_ai adds reactive maneuvers, countermeasures and launch behavior.

## Player defaults and configuration

[`config/hf_ai.json`](config/hf_ai.json) is the tracked player default profile.
The release pack ships it unchanged, and compiled defaults match it. A missing
file is generated with these values; missing keys or blocks in partial configs
inherit them. Existing explicit settings are retained. Edit
`Modloader/config/hf_ai.json` while the game is closed and restart after edits.

The complete key list (including retired and internal-default keys) is in
[the config reference](../docs/hf-ai-config-reference.md), with behavior
explained in [the tuning guide](../docs/hf-ai-tuning-guide.md).
[Player defaults](../docs/player-defaults.md) explains the shared release policy.
Use top-level `"enabled": false` to disable the whole mod; `{}` uses the active
player profile and is not a vanilla/off configuration.

## Behavior

- **Universal aim dodging (`tactics.aim_*`):** during per-ship staggered awareness
  windows, enemy and allied AI ships check all hostile shooters (both the player
  and AI ships) and slip off the closest active aim ray (`aim_chance` roll per
  window).
- **Offensive early fire (`tactics.aim_fire_*`):** once per awareness window, an
  AI ship in range rolls `aim_fire_probability` for 1 s of widened group-0 range
  permission. Since v0.10.12 the native controller retains actual barrel alignment,
  friendly-fire, ammunition and burst-start checks; the mod never writes player
  fire bytes. Native latched bursts can outlast that permission and skip later
  checks, so this does not guarantee avoiding friendly fire. Group 1 is unchanged.
- **Dive avoidance (`tactics.dive_*`):** sidesteps when its target stays above
  or below it for `dive_dwell_s`, retrying after `dive_cooldown_s` if a roll is
  declined. Aim dodging takes priority over dive avoidance.
- **Missile defense (`ai_flares`):** locked-on threats trigger flare bursts and
  evasion; stray threats trigger evasion only. Preventive flare curtains use a
  one-time ETA estimate minus 4 s, clamped to a 1–8 s wait, checking `launch_flares_min_rack` before
  every drop and retreating within `launch_flares_evade_range`. Ships without
  flare launchers still evade missiles in all three stages.
- **Zenith bursts (`ai_missile_dump_*`):** enemy and allied AI ships can empty
  their Zenith racks in a rapid burst instead of waiting for the stock 2 s cooldown.
- **Tactical carrier groups:** `plane_group_per_launch` and
  `plane_group_launch_delay_seconds` space ready aircraft launches for enemy and
  allied AI carriers in normal `Combat.seria`. Size 1 restores stock cadence.
- **Debug overlay feed:** publishes each AI ship's tactics/flare state to the
  `Local\hf_ai_debug_state_v1` shared-memory table read by hf_ally's
  `debug_overlay`.

The top-level `twr_cutoff` (default **2.0**) is the sole maneuver capability
threshold, reading live strategic/Shipworks TWR (`creature +0x160c`). Ships below
it do not use tactics or missile maneuvers, though flare releases remain active.
`0.0` disables the gate. Engine boosts (`dive_boost`, `aim_boost`, `panic_boost`,
`launch_flares_boost`) work independently of whether `ai_flares.enabled` is on.

## Build and validation

```bash
cargo xwin build --release --locked --no-default-features --target x86_64-pc-windows-msvc
```

Use one hf_ai DLL at a time. Keep the QoL flare crash fix enabled. A separate
QoL `ai_flares` implementation must be off because it patches the same sites.

See [CHANGELOG.md](CHANGELOG.md) for v0.10.12 corrections. Cross-building is not
combat validation; in-game validation remains pending. Existing host math harnesses
contain zero tests. Page checks and Rust `catch_unwind` do not catch native access violations.

## Retired and renamed settings

See [docs/hf-ai-config-reference.md](../docs/hf-ai-config-reference.md#retired-settings)
for the complete list of retired keys and internal tuning parameters omitted from
the default `hf_ai.json` preset in v0.10.11.

| Former key | Status |
|---|---|
| `ai_flares.launch_flares_shelter`, `launch_flares_shelter_range` | Retired in v0.10.11. Ships without flares evade missiles directly. |
| `tactics.dive_under_chance`, `dive_under_dwell_s` | Merged into `tactics.dive_chance` (`0.8`) and `tactics.dive_dwell_s` (`0.5`) in v0.10.11. |
| `tactics.min_accel`, `dive_min_accel` | Retired in v0.10.4. Use top-level `twr_cutoff`. |
| `tactics.k_center`, `band_min`, `band_max`, `half_width`, `outrange_extra`, `blend_weight`, `strafe_min_s`, `strafe_max_s`, `retreat_tangent`, `strafe_enabled` | Removed with optional positioning in v0.10.6. |
| `tactics.apply_to_allies` | Renamed to `tactics.allies` in v0.10.5 (old name still accepted as an alias). |
