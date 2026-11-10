# HighFleet Allies v@PACK_VERSION@

- **Fixed: ship bombers caused a huge performance drop.** One of the bomber checks was attached to a place in the game code that runs once for every part of a ship, and a large ship has hundreds of parts. Each run redid the whole situation check from scratch — target, speed, where to fly — hundreds of times per frame. It now runs once per ship.

## Debug overlay settings

In `Modloader/config/ally.json`, under `debug_overlay`:

| Setting | Default | Effect |
|---|---|---|
| `enable` | `false` | Show the overlay. |
| `circles` | `[250, 500, 1000]` | Ship-ring radii in game units; `[]` hides them. |
| `ai_waypoint` | `true` | Show AI destinations and distances. |
| `prediction_marker` | `false` | Show estimated gun-aim markers for supported cannon ammo. |
| `pointer_length` | `300` | Native aim-line length and bearing-marker distance in game units; `0` hides both. |

Keep top-level `enabled: true`; restart after editing. `ally_reticle_scale` controls ally target boxes separately. With hf_ai loaded, ship labels also show its AI state. AP, guided shells and rockets have no aim prediction.

## Install

HighFleet **Steam 1.163**, Modloader v4. Back up `Saves`, extract the ZIP into the game directory, and remove older ally/AI DLLs so only one of each remains. Merge the included configs with your custom settings. Keep the QoL flare crash fix enabled.

Includes **hf_ally v@ALLY_VERSION@**, **hf_ai v@AI_VERSION@**, bundled QoL, presets and the player manual. Bomber fix cross-build validated on 2026-11-10; run a bomber battle to confirm frame times.
