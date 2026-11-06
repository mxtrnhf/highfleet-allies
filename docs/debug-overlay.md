# Battle debug overlay

`hf_ally` 0.14.14 added the optional tactical-battle overlay; 0.14.15 adds the ring distance
labels and the AI waypoint display. It is off by default and is independent of `spawn_ally`.

```json
"logging": {
  "debug_overlay": true
}
```

The old top-level `"debug_overlay": true` is still read and moved into the
`logging` block when the config is parsed.

## What it draws

For every live ship in the battle (player, allies, enemies, neutrals):

* three concentric rings at **250, 500 and 1000 world units** around the hull centre, so the
  rings scale with the battle camera instead of staying a fixed pixel size;
* each ring carries its radius in world units (`250u`, `500u`, `1000u`) just right of the ring line;
* one short label above the ship: side/role tag, the raw `+0xbdc` driver state and its name;
* for AI ships, the current steering waypoint (`+0xbf0/+0xbf4`): a dotted lead-in from the hull,
  a small orange marker, and `WP <distance>u` at the point. When the waypoint is off-screen,
  `DEST <distance>u` appears under the ship label instead.

| Label | Meaning |
|---|---|
| `P` | the ship the player flies (`+0xd70 == 1`) |
| `A` | friendly AI (`+0xad4 > 0`) |
| `E` | enemy (`+0xad4 < 0`) |
| `N` | neutral (`+0xad4 == 0`) |

State names come from `hf_notes/docs/findings/ai/ai-behaviour-states.md`:

| `+0xbdc` | Label |
|---|---|
| 0 | `HOLD` |
| 1 | `ENGAGE` |
| 2 | `ENGAGE*` (engage/fixed, the driver no longer re-derives the state) |
| 3 | `MOVE` (move to the stored point and wait) |
| 4 | `LEAVE` (go to the arena edge) |
| 5 | `PARKED` |
| 6 | `DEAD` |
| 7 | `ALARM` |

Examples: `E 1 ENGAGE`, `A 0 HOLD`, `P 2 ENGAGE*`.

When **hf_ai v0.10.9 or newer** is loaded, a second line shows its own per-ship layer:

| `AI ...` | Meaning |
|---|---|
| `AI STOCK` | hf_ai's tactic layer left the stock steering alone |
| `AI DIVE` | “deny the dive” sidestep |
| `AI DIVE-SKIP` | the dive cone was detected but the chance roll declined the dodge |
| `AI AIM` | aim-awareness sidestep (or its early-fire window) |
| `AI FLARES` | an ai_flares flare burst is running |
| `AI EVADE` | missile evasion is steering the ship |
| `AI PANIC` | panic evasion (dash at `panic_boost`, full steering takeover) |
| `AI ROLL` | barrel-roll evasion |
| `AI CURTAIN` | the pre-emptive flare curtain is running |

The flare/evasion modes take precedence over the tactics state because they override the
waypoint while active. If hf_ai is not installed, the second line is simply not drawn.

## hf_ai cross-mod state

hf_ally and hf_ai are separate DLLs, so the state travels through a small named shared-memory
mapping created by hf_ai at start-up:

* name: `Local\hf_ai_debug_state_v1` (mapping format version 1);
* writer: `ai/src/debug_state.rs` (hf_ai v0.10.9+);
* reader: `ally/src/debug_state.rs` (hf_ally v0.14.14+);
* fixed table: 256 entries, `{ creature u64, game clock f64, tactics u8, flares u8 }`;
* hf_ally ignores entries older than 2 game-seconds, so a crashed/absent hf_ai or a battle
  restart cannot show stale state.

Both DLLs run on the game thread. hf_ally only opens the mapping read-only and retries every two
seconds until hf_ai has created it, so load order does not matter. The structs are deliberately
duplicated in both crates; change both together and bump the mapping name if the layout changes.

## Native implementation

All addresses are Steam 1.163, image base `0x140000000`.

| Purpose | Address / field |
|---|---|
| Per-frame battle HUD pass that already owns the camera projection | `0x14029c5c0` (`afterBattleStars??`) |
| Its call from `combatEvents`; the mod wraps this call | `0x14029a8c1` (5-byte `E8 rel32`) |
| World point -> screen; `(Vec2*, bool absolute, float zoom)` | `0x1401adfc0` |
| Floor the projected point | `0x1402e51c0` |
| Hull centre of mass used by the AI | creature `+0x1840` / `+0x1844` |
| AI steering waypoint (final output of the AI tick) | creature `+0xbf0` / `+0xbf4` |
| Side float / role / driver state | creature `+0xad4` / `+0xd70` / `+0xbdc` |
| Global creature vector | `0x143faa5d8..0x143faa5e0`, stride `0x18`, pointer at `+8` |
| Screen-space circle drawer used for the rings | `0x140244b00` (`Gfx_DrawCircleClipped`) |
| Its line primitive (screen-space when `alreadyScreen = 1`) | `0x140248ba0` (`Gfx_DrawWorldLine`) |
| Battle HUD enable byte | `0x143a11961` |
| PLAYER-label font / style / render | `flash_large_shadow`, `0x1401a8710`, `0x14016eb40` |

The rings are built by the game's own `Gfx_DrawCircleClipped`, which emits `Gfx_DrawWorldLine`
segments at screen coordinates (1 pixel wide, solid). No new texture is shipped and the rings stay
thin at every camera zoom. Ring colour is the game's friendly cyan, hostile red or neutral lime,
with per-ring alpha.
The labels reuse the same `flash_large_shadow` font, scale, blend and style save/restore recipe as
the pre-combat `PLAYER` label in `ally/src/lib.rs`.

`ally/src/ally_reticles.rs` is the single owner of the `0x14029a8c1` call site. It runs the native
target/spawn-box pass first, then the ally reticles (if enabled), then this overlay. That avoids
two mods or two features detouring the same five bytes.

Relevant code:

* `ally/src/debug_overlay.rs` -- projection, rings and labels;
* `ally/src/ally_reticles.rs` -- shared battle HUD post-pass;
* `ally/src/lib.rs` -- `logging.debug_overlay` config key and install condition.

## Validation status

* `cargo xwin build --release --target x86_64-pc-windows-msvc` succeeds (0.14.14).
* **Not yet validated in game.** The build proves the layout and hook compile; it does not prove
  that the circle drawer, font and hook placement render correctly under every zoom, pause state
  and end-screen transition.
* Things to check in game:
  1. rings are centred on hulls and shrink/grow with zoom;
  2. labels stay above the correct ships while the camera pans;
  3. labels do not survive into the city view / Esc menu / end screen;
  4. the game's own missile circles and target boxes still look normal (the line primitive and font
     style are shared native objects; the font style is saved and restored after each draw);
  5. `logging.debug_overlay: false` draws nothing and keeps the old reticle-only behaviour;
  6. with hf_ai loaded, the second line tracks `DIVE` / `AIM` / `EVADE` / `ROLL` as those stages
     start and end, and the line disappears when hf_ai is removed;
  7. ring labels stay attached to the correct ring at different zoom levels, and the waypoint
     marker/dotted lead-in end where the ship is actually steering.

## Development log

### 2026-11-03 -- initial implementation

* Base repository: `highfleet-allies` `main`, commit `279c0d4` (release v0.14.13 notes).
* Inspected the existing `ally_reticles` battle HUD hook and `hf_ml` simulator overlay ideas.
  The simulator draws its overlays in HTML/canvas, so only the visual idea (rings plus
  ship-following labels) carried over; the in-game projection and drawing code are from the 1.163
  native HUD.
* Used Ghidra to confirm:
  * `BattleHud_DrawBodyMarkers` (`0x14029ae40`) draws `sign_circle` (`DAT_14800eac8`) at a body's
    screen position with `Gfx_QuadRenderEx(..., 1.0, 0.0)`;
  * `sign_circle` is declared in `Midground2.res` as a 33 x 33 ring with hotspot `16,16`, but
    scaling that tiny sprite to hundreds of pixels made the debug rings very thick, so the final
    code uses `Gfx_DrawCircleClipped` (`0x140244b00`) instead;
  * `Gfx_DrawCircleClipped` takes screen coordinates and a pixel radius, emits
    `Gfx_DrawWorldLine` segments (1 pixel wide) and clips to the screen, so the rings stay thin;
  * `afterBattleStars??` (`0x14029c5c0`) uses `ship + 0x1840/+0x1844` plus the native `+0x240`
    popup offset for the target box and calls `Vec2_WorldToScreen` with zoom `1.0`.
* Chose three fixed world-space radii (250/500/1000) so the debug rings show comparable distances
  between ships; they are not tied to weapon range, so they remain useful before the AI has picked
  a target.
* Added the overlay to the existing `ally_reticles` hook instead of adding a second detour at
  `0x14029a8c1`. Both features can now be enabled separately, and only one set of five stolen
  bytes exists.
* Version bumped to 0.14.14 in `Cargo.toml` and `Cargo.lock`; the player preset gains the disabled
  `logging.debug_overlay` key.

### 2026-11-03 -- hf_ai state line

* Read the hf_ai layer: `tactics_math::State` (`Stock`, `Panic`, `Dive`, `DiveSkip`, `Aim`) and the
  `ai_flares` modes (`flares`, `evade`, `panic`, `roll`, plus the pre-emptive curtain).
* Added a versioned shared-memory channel because the two mods are independent DLLs and neither
  module name nor load order is stable. hf_ai v0.10.9 creates `Local\hf_ai_debug_state_v1` and
  writes the tactics/flare bytes per AI ship; hf_ally 0.14.14 opens it read-only and draws the
  second label line.
* Flare/evasion modes win over tactics in the label, matching hf_ai's own precedence (`ai_flares`
  overrides the tactics waypoint while it is active).
* hf_ai clears the table when the game clock jumps backwards (new battle); hf_ally additionally
  treats entries older than two game-seconds as absent.
* Version bumped to hf_ai 0.10.9. Still cross-build only; in-game verification is pending.

### 2026-11-03 -- ring distances and AI waypoint

* Added a `250u` / `500u` / `1000u` label just right of each ring. Labels are skipped for rings
  smaller than 30 screen pixels so the ship's own label stays readable when zoomed out.
* Added the AI steering waypoint from creature `+0xbf0/+0xbf4` (the final waypoint after the AI
  tick and hf_ai's overrides). It is drawn as a dotted lead-in plus a small orange target ring.
* The waypoint text shows `WP <distance>u` when the point is on screen; when it is off-screen,
  `DEST <distance>u` is added under the ship's state label so the destination is still readable.
* Waypoint and dotted lead-in use the same `Gfx_DrawCircleClipped` helper (small circles), so no
  additional native line-drawing ABI is involved.
* hf_ally version bumped to 0.14.15.
