# hf_ai v0.10.7

A tactical AI layer for **HighFleet Steam 1.163**, loaded as a standalone DLL by
logdot's Modloader v4. The stock AI still selects targets and computes movement;
hf_ai adds reactive maneuvers, countermeasures and launch behavior.

## Player defaults and configuration

[`config/hf_ai.json`](config/hf_ai.json) is the tracked player default profile.
The release pack ships it unchanged, and compiled defaults match it. A missing
file is generated with these values; missing keys or blocks in partial configs
inherit them. Existing explicit settings are retained. Edit
`Modloader/config/hf_ai.json` while the game is closed and restart after edits.
JSON key order does not matter, but keys must remain in their correct objects.

The complete key list is in [the config reference](../docs/hf-ai-config-reference.md),
with behavior explained in [the tuning guide](../docs/hf-ai-tuning-guide.md).
[Player defaults](../docs/player-defaults.md) explains the shared release policy.
Use top-level `"enabled": false` to disable the whole mod; `{}` uses the active
player profile and is not a vanilla/off configuration.

## Behavior

- **Aim dodging:** during awareness windows, a ship on the player's aim line
  steers off it, with optional early fire. Uses `tactics.aim_*`.
- **Dive avoidance:** sidesteps when its target stays above or below it long
  enough; uses `tactics.dive_*`. Aim dodging takes priority over this maneuver.
- **Missile defense:** locked-on threats can trigger flares and evasion; stray
  threats trigger evasion only. Preventive curtains can drop flares and move
  away from missiles or shelter behind another ship's flares.
- **Zenith bursts:** `ai_missile_dump_*` can shorten the pause between missiles.
- **Tactical carrier groups:** `plane_group_per_launch` and
  `plane_group_launch_delay_seconds` space ready aircraft launches. Enemy and
  allied AI carriers only, normal `Combat.seria`; strategic/manual paths keep
  stock behavior. Size 1 restores stock cadence.

The top-level `twr_cutoff` is the sole maneuver capability threshold. The player
default is **2.0**: ships below it do not use tactics or missile maneuvers;
ships at or above it remain eligible. `0.0` removes this restriction. Individual
switches, target checks and ground/arena safety limits still apply. Locked-threat
flare bursts and preventive flare drops remain available below the cutoff.
Carrier plane groups are independent of TWR.

`tactics.enabled` controls aim/dive steering and early fire; `tactics.allies`
includes allied AI. `ai_flares.enabled` controls the missile-defense subsystem,
and `ai_flares.allies` independently includes allied AI. The player-controlled
ship is excluded. Missile panic has priority over aim and dive dodges. If no
reactive dodge acts, the stock waypoint is retained at every target distance.
Gun range is still tracked for early fire.

There is no optional preferred-distance band, circling, periodic strafe reversal,
or close-range positioning retreat. This routine was removed in v0.10.6.
Flare-curtain retreat/shelter remains part of missile defense.

## Build and validation

```bash
cargo xwin build --release --locked --no-default-features --target x86_64-pc-windows-msvc
cargo xwin test --release --locked --target x86_64-pc-windows-msvc --no-run
```

From the repository root, `python3 tools/check_player_defaults.py --offline`
runs host configuration/math tests without loading a DLL or starting the game.
Full pure-math suites can also use rustc directly, for example:
`rustc --edition 2021 --test src/tactics_math.rs -o /tmp/tactics_test`.
The player performs gameplay testing; build/test compilation alone does not
validate hooks in a running game.

Use one hf_ai DLL at a time. Keep the QoL flare crash fix enabled. A separate
QoL ai_flares implementation must be off because it patches the same sites.
Game addresses and hook bytes are for Steam 1.163; code checks protect the sites
before installation. RE findings live in the workspace's `hf_notes/` repository.

## Deprecated and renamed settings

Legacy keys below are ignored and omitted from generated configs.

| Former `tactics` key | Status |
|---|---|
| `min_accel`, `dive_min_accel` | Retired in v0.10.4. Use top-level `twr_cutoff`. |
| `k_center`, `band_min`, `band_max`, `half_width`, `outrange_extra` | Removed with positioning in v0.10.6. |
| `blend_weight`, `strafe_min_s`, `strafe_max_s`, `retreat_tangent`, `strafe_enabled` | Removed with positioning in v0.10.6. |

`tactics.apply_to_allies` was renamed to `tactics.allies` in v0.10.5. The old
spelling remains accepted as a compatibility alias; use only one spelling.
`tactics.allies` and `ai_flares.allies` both default to true in the player profile,
while their scopes remain independent.
