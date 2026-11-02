# hf_ally v0.14.5 tester build (candidate GUI fixes)

**Status:** local tester DLL only. This is not a GitHub release and is not part of the
release pack. It has **not been validated in game** yet.

The changes are limited to hf_ally's own GUI handling: checking current native membership
before reading cached button addresses, reusing still-live root children rather than
reviving detached nodes, and repainting/loading one native-owned sprite per button.
Unchanged card layouts receive a rate-limited health check (one per second, with at most
eight consecutive repair attempts). Larger GUI lists are scanned with a count-derived
bound; an incomplete scan does not prove that cards or buttons have disappeared.
The health repair targets membership, active state and sprite loading; it does not force
opacity or shader state. Opacity-zero counts are included in the build diagnostic instead.
At the hard safety limit of 4096 GUI list nodes, placement/recovery pauses and logs an
incomplete scan rather than guessing that missing cards have gone. Persistent incomplete
scans can therefore leave old text visible until an explicit exit; send those logs.

These are **not a shader fix**, and no shader root cause has
been confirmed. Do not assume this build fixes a problem that also happens without hf_ally.

Validation: locked Windows release cross-build and Clippy for both default and `megalog`
configurations pass with warnings. `cargo fmt --check` is not clean: broad formatting
differences were already present in 11 crate files before this work; unrelated formatting
was not changed. No new regression tests were written, and no in-game testing was performed.

## Install (standalone DLL)

Requirements: HighFleet **Steam 1.163** and **Highfleet-Modloader v4**. Close the game
completely before changing files.

1. Back up your `Saves` folder.
2. Move your current `hf_ally_v*.dll` out of `Modloader/mods` into a backup folder
   **outside** `Modloader/mods` (for example next to your saves or in a separate
   `ally-backup` folder).
3. Move any other `hf_ally*.dll` out of `Modloader/mods` too. Do not leave backup DLLs
   in that folder: the loader can load those as well.
4. Put `hf_ally_v0.14.5.dll` into `Modloader/mods/`.
5. Keep your existing `Modloader/config/ally.json`, `hf_ai.json`, QoL DLL/config and
   `Media` folder. Do not replace them and do not update hf_ai or QoL for this test.

To roll back, close the game, move `hf_ally_v0.14.5.dll` out of `Modloader/mods`, and
restore the backed-up hf_ally DLL. Keep exactly one hf_ally DLL installed.

## Logging

Use the existing nested `"logging"` block in `ally.json`. `"debug_log"` is already `true`
in the player preset; set `"gui_inspect"` and `"log_precombat"` to `true`:

```json
"logging": {
  "debug_log": true,
  "gui_inspect": true,
  "log_precombat": true
}
```

Keep the other logging keys as they are. This is deliberately noisy: `debug_log` enables
the megalog, watchdog and crash dump, `gui_inspect` logs GUI clicks and ammo-selector
rebuilds, and `log_precombat` logs the pre-combat GUI lists. Keep `spawn_ally` and
`ally_toggle` enabled as in your normal config so the ALLY buttons and click inspector
are active. Restore quieter settings after the test.

## What to test

### 1. Shader files and comparison

If your missing-GUI case involves edited shaders, send the **edited shader files** and the
**matching original files**, or a clear original-versus-edited diff. State which files
they replace and where they are installed. Shader compatibility is still unverified;
without both versions the symptom cannot be separated from a shader problem.

### 2. Screenshots: icons versus labels

Capture the pre-combat screen before and after the GUI disappears. These controls are
only intended for pre-combat; their disappearance after starting battle is normal. The
ALLY/plane buttons are native GUI icons, while the nearby `PLAYER` and loadout text is
drawn separately. For each screenshot, state whether the **icon/button**, the **text
label**, or **both** are missing, and whether the remaining element is blank, black,
shifted or overlapping.

### 3. Blind-click behaviour

When an element is invisible, click once where it should be and report what happens:
nothing, the expected ally/plane action, or a different action. Do this only where the
result is harmless, and note whether an invisible control is still clickable or its click
area has moved. Also note whether the element reappears after the click.

### 4. Reordering, RETRY and a long session

- Drag/reorder the pre-combat ship cards. Check that ALLY buttons and plane buttons follow
  their cards, labels stay beside the right buttons, and nothing overlaps or keeps the old
  position.
- Use **RETRY** in Shipworks, and enter/leave the pre-combat screen several times. Look for
  duplicated, stale, hidden-but-clickable or wrongly placed buttons.
- Play several battles in one session, including retreats, save/load, the Esc menu and at
  least one longer session. Note any gradual GUI loss, accumulating elements or crashes.

## Logs and report

Send the newest `Highfleet_*.log`, `hf_ally_megalog_*.log`, and any
`hf_ally_crash_*.dmp` from `Modloader/logs`. Dumps may contain paths and campaign data;
delete them after sending. Confirm the log banner says `hf_ally v0.14.5 (build ...)`.

Include:

- game and Modloader versions, and the QoL DLL you use;
- the exact `ally.json` logging block and any non-default settings;
- whether edited shaders are installed, with the files/diff;
- reproduction steps and timing, screenshots, and the relevant log section;
- whether the problem also occurs with the old hf_ally DLL or with hf_ally disabled.

Report the result even if nothing changed: a negative result is needed before any fix is
described as confirmed.

## Native contracts checked for this build

Read-only checks against the Steam 1.163 Ghidra program confirmed:

- `SetBackground_` (`0x14001be50`) allocates and appends a Sprite; its final argument is
  an insertion position, not a replacement slot. Repeated calls do not replace old sprites.
- `ObjectArray_DeleteAll` (`0x14004bb10`) clears the Body sprite vector through each
  Sprite's native deleting destructor, preserving native allocation ownership.
- `setBodyOwnerString` (`0x1402e2030`) can retarget an existing Sprite by resource name;
  `GUI::Load` (`0x140163110`, virtual slot `+0x88`) delegates to `Body::Load`
  (`0x14004a040`) and loads the sprite resources. The call historically called
  "refresh" in this mod is a resource load, not merely a request to redraw.
- The native setter itself skips an unchanged resource name; no second Rust sprite-state
  cache is needed. Invalid sprite-vector headers are rejected before native mutation/load.
- The GUI registry is separate from the root's child list. A retained pointer must be
  validated against live native membership before use. Detached objects must not simply
  be reattached: `Node::add` does not remove a node from the native pending-deletion queue.
- New elements are registered synchronously by the existing initializer path:
  `0x14016a350` calls `CreateShell_` (`0x14004edd0`), whose call at `0x14004efda`
  invokes virtual slot `+0x30`. For GUI this is `Gui_LoadAndInitElements`
  (`0x1401636b0`), which calls `LoadAllGuiElementsIntoGlobal` (`0x140224ae0`).
  The latter stores the registry cell at `+0xba8`. No new registration call is needed.

Covered layouts remain derived from the pinned `highfleet-rs` types. No game-owned Rust
string, vector or object is copied into an owning value or destroyed with Rust drop.

## Evidence from the reporter's v0.14.4 logs

The 2026-11-02 capture in `mail/gui_troubles/` narrows the lifecycle failure considerably:

- At 10:29:27, the first pre-combat build registers `ALLY_TOGGLE_2` at `0xe1677090`
  (main log, line 739). The user clicks an ALLY button and starts combat; the mod hides
  its three toggles normally.
- At 10:31:01, the next pre-combat registry contains 45 elements and none of those ALLY
  buttons (line 10490). Placement starts and an access violation follows in
  `on_precombat B2`: the failing read address is `0xe1677ad0`, exactly the old button
  pointer plus the `0xa40` identifier offset (lines 10493–10496; megalog lines 619–624).
- This strongly implicates stale retained GUI memory after scene teardown, not merely a
  shader making a live button transparent. Reading a cached name is not a lifetime check:
  current native membership must be established before dereferencing the cached pointer.
- The pre-combat registry has only 33–45 elements, below the old scan limit. All four
  ships report zero aircraft, so the absent plane buttons in this capture are intentional.
- Other missing sprite/sound resources are logged, but none of the custom button's
  `order_*` resources is reported missing. The logs do not prove a shader root cause.

The reporter's executable filename is immaterial; the loader identifies Steam 1.163.
The fix still requires an in-game re-test of consecutive battles and scene transitions.
