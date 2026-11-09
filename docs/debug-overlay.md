# Battle debug overlay

`hf_ally` 0.14.14 added the optional tactical-battle overlay; 0.14.15 added the ring distance
labels and the AI waypoint display. Since v1.0.1 it is configured in its own settings block
and gained configurable ring radii, an on/off switch for the waypoints and the hostile lead
(aim-assist) markers. It is off by default and independent of `ally_reticle_scale`.

```json
"debug_overlay": {
  "enable": true,
  "circles": [250.0, 500.0, 1000.0],
  "ai_waypoint": true,
  "prediction_marker": false,
  "pointer_length": 300.0
}
```

An empty `circles` list draws no rings; more than 8 entries and non-finite or non-positive
radii are ignored. The old `"logging": { "debug_overlay": true }` (and the even older
top-level `"debug_overlay": true`) still enables the overlay with the block's defaults;
an explicit `debug_overlay` block wins over both legacy forms.

## What it draws

For every live ship in the battle (player, allies, enemies, neutrals):

* ring circles at the `circles` radii in **world units** around the hull centre, so the
  rings scale with the battle camera instead of staying a fixed pixel size; ring alpha
  fades outward so the set reads as depth;
* each ring carries its radius in world units (`250u`, `500u`, ...) just right of the ring line;
* one short label above the ship: side/role tag, the raw `+0xbdc` driver state and its name;
* for AI ships with `ai_waypoint: true`, the current steering waypoint (`+0xbf0/+0xbf4`):
  a dotted lead-in from the hull, a small orange marker, and `WP <distance>u` at the point.
  When the waypoint is off-screen, `DEST <distance>u` appears under the ship label instead;
* with `prediction_marker: true`, an estimated barrel-aim marker for each hostile ship
  reachable by the player's first live gun (group 0, else group 1), labelled
  `AIM ~<flight time>s`. Since v1.0.3 it uses the gun's actual world origin, the native
  10-unit muzzle offset, the firing gun's master-root inherited velocity, shell gravity
  and linear damping. It uses the mean launch-speed multiplier **1.09**, because the
  game independently rolls **0.98–1.2 per shell**; see [instruction-level evidence](shell-launch-speed.md).
  The marker represents the required **barrel bearing**, not the shell's future impact
  position: pointing at the latter fails when inherited velocity or gravity matters.
  The target is assumed to maintain its current root-body velocity. The bounded solver
  searches at native 0.02 s steps, interpolates the intercept within the final step and
  rejects solutions beyond the ammo lifetime (search capped at 30 s).
* each solution also draws a bearing circle at `pointer_length` world units from the
  player's hull centre. This replaces v1.0.2's fixed `bounding_radius + 170` placement.
  The ordinary native aiming line uses the same configured endpoint distance while the
  overlay is enabled. **Default: 300 units**, matching the ordinary vanilla call; vanilla
  uses 400 in its alternate HUD mode. Its normal near endpoint is 80 units (shortened
  when necessary). The separate **2,000-unit rocket ray** and the **100-screen-pixel gun
  arrows** are unchanged. Native obstacles can still clip the ordinary line.
  `0` hides the ordinary line and bearing circles, but keeps the distant aim markers;
  negative, non-finite or greater-than-100,000 values fall back to 300. Restart to apply.

### Prediction limits

This is an estimate, not auto-aim. Random speed/spread, recoil, turret traverse/blocked
arcs and target manoeuvres still cause misses; mixed guns are represented by the first
live gun only. Rockets, guided shells, canister, harpoons and **quadratic-drag AP** have
different trajectory rules and intentionally get **no prediction**, rather than a false
straight-shell solution. The native pointer remains available with those loadouts.
Launches reaching the Box2D 2,000 u/s translation cap are also not predicted.

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
| `AI AIM` | aim-awareness sidestep |
| `AI FLARES` | an ai_flares flare burst is running |
| `AI EVADE` | missile evasion is steering the ship |
| `AI PANIC` | panic evasion (dash at `panic_boost`, full steering takeover) |
| `AI ROLL` | barrel-roll evasion |
| `AI CURTAIN` | reserved; the preventive curtain was retired in hf_ai v0.10.18, so only older hf_ai DLLs can show it |

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
| Ship velocity (root body) | `Body +0x130` / `+0x134` (root via the `Node::master` chain, see `bomber.rs::root_part`) |
| Gun groups (first gun for the lead) | creature `+0x13a8` / `+0x13c0`, `Link` entries, gun `Body` at the link target |
| Loaded ammo index of a gun | gun `Body +0x850` (`m_weapon_ammo`) |
| Global ammo list (`Ammo` records, stride `0x188`; muzzle `speed` at `+0x158`) | `0x143a13be0` (`{begin*, end*}` header) |
| Gun world origin, native master-transform helper | `Node_GetWorldPosition`, `0x140048b60` |
| Ordinary aim-ray call patched for `pointer_length` | `0x1402a003f`: `e8 4c 84 fa ff`, target `Hud_DrawAimRay` `0x140248490` |
| Vanilla ordinary/alternate ray range constants | `0x140352830` = 300.0; `0x140352834` = 400.0 |
| Native ordinary ray's near endpoint | `0x140354854` = 80.0 |
| Separate rocket ray (not patched) | call `0x14029fcdd`, range `0x1403548bc` = 2000.0 |
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

* `ally/src/debug_overlay.rs` -- projection, rings, labels, waypoint and lead markers;
* `ally/src/aim_math.rs` -- pure bounded ballistic intercept solver;
* `ally/src/ally_reticles.rs` -- shared battle HUD post-pass;
* `ally/src/lib.rs` -- the `debug_overlay` config block, legacy-key migration and install condition.

## Validation status

* v1.0.3 confirmed working in game by the maintainer on 2026-11-09.
* v1.0.3 default and legacy `megalog` feature builds pass with `--locked`; no dependencies
  updated. Cross-target Clippy completes with 22 existing style/documentation warnings;
  no blanket suppressions or unrelated cleanup. Scoped rustfmt checks pass for the solver,
  overlay and layout file. Crate-wide formatting has existing unrelated differences.
  No regression tests were added (not authorized); the repository has removed its old tests.
* v1.0.1 (settings block, configurable circles, waypoint switch, lead markers) validated in
  game on 2026-11-09 by the maintainer. The v1.0.2 gun-bearing circles are cross-build only;
  in-game verification pending.
* Things to check in game:
  1. rings are centred on hulls and shrink/grow with zoom;
  2. labels stay above the correct ships while the camera pans;
  3. labels do not survive into the city view / Esc menu / end screen;
  4. the game's own missile circles and target boxes still look normal (the line primitive and font
     style are shared native objects; the font style is saved and restored after each draw);
  5. `debug_overlay.enable: false` (and the legacy `logging.debug_overlay: false`) draws nothing
     and keeps the old reticle-only behaviour;
  6. with hf_ai loaded, the second line tracks `DIVE` / `AIM` / `EVADE` / `ROLL` as those stages
     start and end, and the line disappears when hf_ai is removed;
  7. ring labels stay attached to the correct ring at different zoom levels, and the waypoint
     marker/dotted lead-in end where the ship is actually steering;
  8. with `prediction_marker: true`, the `AIM` marker sits where the player's shells actually
     land on a steady-course hostile, reload/ammo changes move it when the muzzle speed
      changes, the bearing circle stays at `pointer_length` world units on the bearing
     to each marker while the camera zooms, and `ai_waypoint: false` / an empty `circles`
      list draw only what is left. Check moving-shooter drift compensation, upward gravity
      compensation, turret offsets, unsupported loadouts and the native pointer at 0/150/300/600.

## Development log

### 2026-11-09 -- corrected ballistic bearing and pointer length (v1.0.3)

* Re-read `FireShell`, `Body::BulletImpact__`, `Node_GetWorldPosition`, `Hud_DrawAimRay`
  and the native HUD caller against live Ghidra. Confirmed the 0.98–1.2 speed roll from
  instructions and constant bytes; [saved evidence](shell-launch-speed.md).
* Fixed the old relative-flight-time/absolute-position mismatch, which aimed along the
  wrong bearing while the shooter moved. Use the gun's world origin and its own native
  master-root velocity rather than the hull centre/root. Compensate the 10-unit muzzle
  offset, mean speed, gravity and damping; reject beyond-lifetime or unsupported solutions.
* `Body_CreateB2Body` (`0x14004b330`) confirms damping 0.05 and gravityScale 1.0;
  `setCertainBodyFloatVals` (`0x140049570`) clears the extra gravity-compensation field
  at `+0x1e4`. Physics reference: `hf_notes/docs/findings/physics/box2d-function-map.md`.
* Added a byte-checked wrapper at the ordinary aim-ray CALL only. Preserve the original
  native ABI/stack arguments, clipping and alpha; do not patch shared float constants.
  Default 300 matches the ordinary call, not the separate 2000-unit rocket ray.
* Checked the consuming crate's pinned `highfleet-rs` `05c6207` and current upstream Ammo
  layout. Use covered fields with compile-time offset assertions; no dependency update.

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
  `ai_flares` modes (`flares`, `evade`, `panic`, `roll`). The old curtain value is now reserved.
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

### 2026-11-09 -- own settings block, configurable circles, waypoint switch, lead markers (v1.0.1)

* `debug_overlay` moved out of the `logging` block into its own settings object with
  `enable`, `circles`, `ai_waypoint` and `prediction_marker`. The old booleans
  (`logging.debug_overlay` and the pre-0.14.19 top-level `debug_overlay`) still load and
  become the block's `enable`; an explicit new block wins over both.
* `circles` replaces the fixed 250/500/1000 radii. Anything from 0 to 8 positive finite
  world-unit radii can be configured; an empty list draws no rings. Ring alpha now fades
  half-strength per ring outward (floored at `0x38`) instead of the old fixed 3-entry table.
* `ai_waypoint: false` turns the waypoint markers and their `DEST` labels off while keeping
  rings and state labels.
* `prediction_marker: true` draws one lead (aim-assist) marker per hostile ship: the target's
  predicted position at the flight time of a straight shell from the player's first gun
  (group 0, else group 1) with its loaded ammo, from the raw root-body velocities and the
  ammo's muzzle speed (`Body::weapon_ammo` `+0x850` indexing the ammo list at `0x143a13be0`,
  `Ammo::speed` `+0x158`; firing-and-shells.md sections 7-8 document the same reads in the
  native AI lead). The shell carrying the shooter's velocity is accounted for; the marker
  is skipped when the target outruns the shell.
* Live ships are collected once per frame into a bounded stack array shared by the drawing
  pass and the player lookup; no per-frame allocation.
* hf_ally version bumped to 1.0.1. Cross-build only; in-game verification pending.

### 2026-11-09 -- gun-bearing circles (v1.0.2)

* Each hostile lead marker now also draws a small "put guns here" ring around the
  player's own ship, on the world bearing from the hull centre to the lead point,
  projected with the same camera as everything else.
* The circle's distance from the hull centre is `Creature::bounding_radius + 170`
  world units, so it scales with the battle camera and clears the hull at every
  zoom; a non-finite hull radius falls back to the plain 170-unit distance.
* The bearing circle is drawn even when the lead point itself is off-screen, so the gun
  angle stays readable while a fast target's marker is off the view. One circle per
  hostile lead solution, in the marker's hostile red.
* v1.0.1 validated in game by the maintainer the same day; v1.0.2 is cross-build only,
  in-game verification pending.
