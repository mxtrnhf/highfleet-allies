# Retired ally settings

These keys were removed from `ally.json`. Older config files may still contain
them; they load without error and the keys are ignored.

## `auto_retreat` (retired in v0.16.0)

`auto_retreat` was only an enable/disable wrapper around `flagship_leaves`.
When false, the mod did nothing when the flagship left alive: the stock game
promoted an ally and the retreat point stayed consumed. That is the vanilla
behaviour the ally mod exists to fix, so the wrapper was retired.

`flagship_leaves` is now always active:

- `"handover"` (default) = the ally is promoted to your ship and the retreat
  point the flagship used up is restored, so the new flagship can still retreat.
- `"retreat"` = the ally is extracted with the flagship and the battle ends as
  a retreat.

An old `auto_retreat` is ignored. Use `flagship_leaves` to choose the behaviour,
or `enabled: false` to turn off the whole mod.

## `ally_reticles` (retired in v0.14.25)

The boolean was the on/off switch for the ally target boxes. It duplicated the
scale setting, so `ally_reticle_scale` now controls both:

- `0` disables the ally boxes.
- A positive finite value sets the box size relative to the native 62 x 62 frame.
- Negative or non-finite values fall back to `0.8` with a warning.

An old `ally_reticles: false` is ignored; set `ally_reticle_scale: 0` instead.
The player's own target box is unaffected.

## `spawn_ally` (retired in v0.14.24)

`spawn_ally: false` used to keep the mod loaded while skipping the AI wingman. It
was a second on/off switch beside `enabled`, and it also gated most ally-side
features: panels, reticles, ship switching, ally ammo, prize sparing, the ship
bomber and the pre-combat UI.

The wingman is the point of the mod, so the key is retired: whenever
`enabled` is true, allies deploy normally. Ship 2 starts switched on; turn its
**ALLY** toggle off on the pre-combat card to skip the wingman, or use
`enabled: false` to turn off the whole mod. An old `spawn_ally: false` is
ignored and no longer suppresses allies.

## `ally_toggle` (retired in v0.14.24)

`ally_toggle: false` removed the pre-combat **ALLY** buttons. It was not just an
ally-button switch: it also disabled the carrier plane-loadout buttons, the
`PLAYER` label and the per-frame GUI handling for that screen. The pre-combat UI
is now always installed when the mod is enabled; there is no reason to hide it
while still deploying allies. An old `ally_toggle: false` is ignored.

## `max_enemy_ships` (retired in v0.14.22)

The old patch changed the immediate in `cmp qword [0x147fc75b0], 5` at
`0x1402caaa0`, inside **`SCR_BOMBERS`** (`0x1402ca2b0`), the strategic bomber
scene setup. It capped that scene's enemy list, not normal tactical combat.

Normal tactical combat is `SCR_COMBAT` (`0x1402cdaf0`). Its enemy list is filled
by `Precombat_SplitFleetIntoReserveLists` (`0x1402d9500`), which appends every
valid non-prize enemy ship from the engagement with no total cap. Vanilla
tactical battles therefore have no total enemy-ship limit, and this key did
nothing in them.

The setting was retired rather than repointed: a real tactical total cap would
have to patch the pre-combat list construction, and the vanilla behaviour is to
use the whole engagement fleet. Use `enemy_group_size` to control how many
enemies are active at once.

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
