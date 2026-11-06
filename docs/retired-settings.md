# Retired ally settings

These keys were removed from `ally.json`. Older config files may still contain
them; they load without error and the keys are ignored.

## `plane_bombers` (retired in v0.14.20)

`plane_bombers` was the original one-byte global patch at `0x14000cf16`. It
turned the game's `jnz` into an unconditional jump, sending **every** plane with
a bomb list and a non-zero state straight into the native bomber routine
`0x14000bd50`.

That bypassed the stock early states that let a plane clear its carrier. In the
2026-10-25 incident, LA-29s entered the bombing dive immediately after arming,
struck the deck of the carrier they had just left, and capsized it. The setting
was therefore turned off by default, but leaving the patch available was still a
footgun.

The supported tactical bombing path is `plane_loadouts`: when you choose a
bomb/rocket loadout, the mod watches that plane and switches it to mode 3 /
state 3 only after `plane_attack_delay` seconds and only once it has a target.
That is safer, applies to the loadout you actually selected, and is independent
of this retired global patch.

## `marker_seam` / `marker_delay_ms` (retired in v0.14.20)

The `PLAYER` and plane-loadout labels now always draw at the B23 layer-flush seam
(`0x140309408`). B24 is skipped when `GFX_GLITCH_LEVEL=0`, and B22 can be covered
by the sliding pre-combat doors, so exposing the seam only created broken
combinations. The fixed B23 hook is the only supported choice.

`marker_delay_ms` was a second, time-based hold in addition to the existing
roller-cover check (`doors_closed()`), which already hides the label until the
doors have played. The extra delay was redundant and has been removed.
