# HighFleet Allies v@PACK_VERSION@

- **Fixed: gunpod plane loadouts could not be picked on the carrier plane buttons.** A loadout that carries no parts of its own — a gunpod, like the 20 mm or 100 mm cannon pods added for planes by QoL/ammo mods — was previously mistaken for the plain "guns only" choice, so the pre-battle carrier buttons never offered it. All of a craft's loadouts are now selectable, gunpods included: the cycle starts at the craft's default gun (vanilla: the 37 mm), gun choices cost no stock, and a launched gunpod plane strafes its target with the selected cannon until its rounds run out, then flies home.

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

Includes **hf_ally v@ALLY_VERSION@**, **hf_ai v@AI_VERSION@**, bundled QoL, presets and the player manual. Gunpod loadout selection cross-build validated on 2026-11-11; launch a carrier battle with a gunpod choice to confirm it arms and attacks.
