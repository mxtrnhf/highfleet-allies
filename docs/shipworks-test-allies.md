# Shipworks test allies (hf_ally v1.2.2)

**v1.2.2:** CLEAR sits immediately right of ADD and empties the roster without
leaving Shipworks or changing the edited ship. It also removes the roster's
temporary snapshots and any full-list/capture-error message. You can ADD again
immediately. CLEAR has the same native Shipworks styling and its own click debounce.

**v1.2.1 capture correction:** v1.2.0 skipped the native bridge search inside frame
children and incorrectly took the serialized design root from the Creature's
master rather than the bridge's master. Both paths now match the native save
routine. Failed ADD operations also log the specific capture stage or filesystem
error. The reported v1.2.0 failure was observed in
`Highfleet_2026-11-13_13-11-27.log`; its generic warnings did not identify which
capture guard returned. The maintainer subsequently confirmed v1.2.1 working in game.

In standalone Shipworks, **ADD** is on the top row, immediately right of **TEST**.
It captures the currently edited ship, including unsaved changes. Each press adds
another copy, and the ship names appear below the button.

Choose the usual TEST enemy settings and press the existing **FIGHT** button.
The edited ship remains your player ship; the added ships join its friendly
pre-combat roster with ALLY enabled. The normal ally controls, aircraft loadout
buttons and combat ship switching still apply.

- Up to **eight allies**, matching the native nine-player-card screen.
- All added ships can deploy together, without the campaign `max_allies` or
  reinforcement delay. Campaign settings and card choices are unchanged.
- Loading, switching designs, RESET, returning from test combat, and RETRY do not
  clear the list. Returning from the Shipworks landing test also preserves it.
- CLEAR or leaving Shipworks clears the list. There is no persistent roster,
  per-ship remove button or additional configuration.
- ADD and CLEAR copy TEST's native backgrounds, dimensions and font. List text uses
  `flash_large_shadow`, the font used for the mod's PLAYER marker.
- A full roster or failed capture is reported below the list. A failed capture
  leaves existing entries intact.

Snapshots are temporary `.seria` files under
`Modloader/cache/hf_ally_shipworks/`, named by process ID and a monotonic capture
number. Only files created for the current roster are removed on CLEAR or exit. A game
crash may leave unused files behind; those files are never used to restore a
roster.

## Native evidence (Steam 1.163)

Read-only verification against the live Ghidra program
`Highfleet.exe.unpacked.exe v1.163`, 2026-11-13:

| Address | Contract |
|---|---|
| `0x14024d960` | Despite its existing `ShipDesign_LoadFromFile` name, its body **saves the current editor design**. Finds a `0x40`-flagged part, gets its owner Creature, collects top-level parts plus the master root, and calls SaveVolume. The mod uses that collection/serialization contract without its unrelated unlock/name/screenshot operations. |
| `0x14024d9c4..0x14024da02` | When an editor Body is a frame (`code & 0x1f == 0x1f`), the native bridge search walks its immediate children. Frame-contained parts need not be separate global Body-array entries. |
| `0x14024dad5` / `0x14024ddfe` | RDI is the found bridge, R15 its owner Creature. The save checks the **bridge's** master at `[RDI+0x88]` and appends that Link via `LEA R8,[RDI+0x80]`; it does not append the Creature's master. Reverified against live Ghidra for v1.2.1. |
| `0x14024ecc0` | SaveVolume(path, name, Link-vector): temporarily reparents roots/joints into a native Node, serializes, restores them to the global root, destroys its own temporary Node. Reads but does not own/reallocate the caller's Link vector. |
| `0x140194bd0` / `0x140194d20` | Native Link construction/registration and destruction/unregistration. Snapshot Links use fixed Rust-owned plain storage, never a copied owning `Link` or `CVec`. |
| `0x14013d807..0x14013d8f8` | FIGHT allocates Node `0x128`, loads its test volume (`0x140225930`), calls virtual `+0x80` with zero, assigns recursive random IDs (`0x140280840`), calls virtual `+0x88`, resolves the ship (`0x14021b070`), recalculates stats (`0x14007fcf0`), checks thrust (`0x140073450`), sets morale `+0x1a20 = 10` and side `+0xad4 = 1`. Added snapshots follow the same sequence. |
| `0x14013dc9d..0x14013dcbf` | Creates/initializes the friendly test Escadra and attaches the current player's volume using its native virtual `+0x50`. |
| `0x14013dcc2` | Hook immediately after that attachment, before fleet traversal. RDI is the friendly Escadra. Replay the complete seven-byte `MOV RSI,[RDI+0x100]`; no flags are consumed before the following native CMP. |
| `0x14025afd0` | SetLevelSeriaToLoad(path, flags). Entry hook preserves arguments, replays two complete home-space stores, and pads entry RSP by eight before the preserved callback. `Combat.seria`, test `City_generated.seria`, and return `Editor.seria` belong to the same session; another level ends it. |
| `0x1401351b7..0x1401353d3` | LANDING test sets the editor-test flag `0x147fc6fb6` and requests **City_generated.seria**, not a hypothetical Landing.seria. |

New native function entries and both hook sites are byte-checked at install.
Existing complete-list GUI membership/root checks protect retained GUI addresses;
no retained editor Creature/part pointers survive a callback. Native GUI
constructors, initializers, string assignment and resource loading own all
game-side allocations. Covered offsets/sizes derive from the pinned highfleet-rs
layouts; `Creature+0xa90` (design name), `+0x1a20` (initial morale), editor globals
and GUI geometry `+0x218/+0x21c` remain locally documented RE contracts.

## Validation

Windows cross-build succeeds. The maintainer confirmed v1.2.1 working in game.
**The new CLEAR button needs in-game verification**; compilation alone does not
validate its placement or behavior. No regression tests were added.

Release Windows Clippy with `megalog` completes with the existing 22 warnings
(none in the new feature files). Changed Rust files pass targeted rustfmt checks;
full `cargo fmt --check` still reports pre-existing differences in other files.
The no-default-features local build is
`ally/target/x86_64-pc-windows-msvc/release/hf_ally_v1.2.2.dll`.

Manual pass on Steam 1.163:

1. Open Shipworks. Check ADD/CLEAR alignment and fonts at the supported screen sizes.
2. ADD one saved design, modify it without saving, and ADD again. Check both names
   and that the two snapshots retain their distinct builds.
3. Load a different design, switch designs and RESET; the list must remain.
4. FIGHT with two or more entries. Check the current design remains the player
   ship and every added copy is friendly, AI-flown and switchable.
5. Retry, then return to Shipworks; the list and snapshot builds must remain.
6. Run/return from LANDING; the roster must remain.
7. Fill all eight slots and attempt another ADD; no ninth ally should be added.
8. Leave Shipworks, re-enter, and check the list is empty and the session files
   were removed. Start a campaign fight and verify its previous ally choices,
    `max_allies` and deployment delay are restored.
9. ADD several ships, then CLEAR. Check that names/status disappear, snapshots are
   removed and the edited ship is unchanged. ADD another design immediately and
   FIGHT; only the new entry should join. Also CLEAR an empty/full/error roster.
