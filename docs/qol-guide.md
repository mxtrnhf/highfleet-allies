# QoL settings: refuel, speed and elevation (`Modloader/config/qol.json`)

The `highfleet_qol.dll` in the release pack is [logdot's highfleet-qol](https://github.com/logdot/highfleet-qol) `main` plus our manual elevation (source in [`../qol`](../qol)). **logdot's stock QoL release (v1.3.0) works fine with our mods; it just does not have the three sections below.** They are included until the changes are merged into a release of the main repo, then use his release.

| Section | What it does | Where it comes from |
|---|---|---|
| `refuel` | how fast fleets refuel at stations | logdot's `main` (our PR #19, merged, no release yet) |
| `speed` | the map cruise-speed formula | logdot's `main` (our PR #19) |
| `elevation` | raise/lower the floor of a held part in the ship editor | **our PR [#20](https://github.com/logdot/highfleet-qol/pull/20), still a draft** |

## Getting the settings into your file
* **No `qol.json` yet:** start the game once with the DLL; it writes a full `qol.json` that already has all keys and all three sections (vanilla values, elevation off), or copy `Modloader/config/qol.full.json` from the pack to `qol.json`.
* **You already have an older `qol.json`:** the DLL does **not** rewrite it. Open `Modloader/config/qol.full.json` from the pack (every key of this build, defaults) and copy what you need into your `qol.json`: at least the three sections `refuel`, `speed`, `elevation`, next to the other top-level keys. Missing sections simply take the defaults, so nothing breaks if you skip them.
* Edit with the game closed; settings are read once at start-up. JSON has no comments and no trailing commas.

## `refuel`
```json
"refuel": { "player_rate_multiplier": 1.0, "ai_rate_multiplier": 1.0 }
```
Multiplies the refuelling rate at stations. `1.0` is the game's rate, `10` is ten times faster. A huge number just means "fill up at once"; you never get more than the tank and the station stock allow. `ai_rate_multiplier` is for AI fleets (they use the same code). `0` never refuels, negative values drain fuel. 1.163 only.

## `speed`
```json
"speed": { "preset": "vanilla", "coefficient": 50.0, "subtract": 1.0, "exponent": 0.5, "max_kmh": 0.0 }
```
The map cruise speed is `coefficient * max(TWR - subtract, 0) ^ exponent` in m/s (the editor shows km/h = m/s x 3.6). TWR is the ship's thrust-to-weight ratio.

| `preset` | Formula | Meaning |
|---|---|---|
| `vanilla` | `50 * (TWR - 1) ^ 0.5` | the game's own 1.163 curve (default) |
| `modern` | same | the 1.163 curve, explicit |
| `classic` | `25 * TWR` | the 1.151 curve: speed grows linearly with TWR |
| `custom` | your three numbers | uses `coefficient`, `subtract`, `exponent` |

`max_kmh` caps the speed in km/h (`0` = no cap). `coefficient` must not be negative; `exponent` must be above 0 and at most 8; `subtract` between -1000 and 1000. An out-of-range value is rejected in the log and the game's curve stays. The game caches each ship's speed: reopen or edit a ship to see a new value. 1.163 only.

## `elevation` (manual ship-editor floors)
```json
"elevation": {
  "enabled": false, "max_floor": 3, "sync_children": true, "child_offset": 0,
  "hint": "native", "keys": { "raise": "U", "lower": "P", "auto": "Backslash" }
}
```
Elevation (floor) decides which parts can block each other's gun arcs and exhaust. With it you can design guns that fire over your own hull. A ship saved with manual floors loads the same without this feature. 1.163 only.

1. Set `"enabled": true`, start the game; the log shows `Elevation prepared: U raise, P lower, Backslash auto (max floor 3, ...)`.
2. In the ship editor pick up a part with the mouse (hold it, do not drop it). **`U`** raises its floor by one, **`P`** lowers it by one, **`Backslash`** gives it back to the game's automatic elevation (then drop the part so the game recomputes it). The keys only react while the game window has focus.
3. A readout on the right shows the keys, the held part, its floor and its mounted modules.

| Key | Meaning |
|---|---|
| `enabled` | turns the feature on (off by default) |
| `max_floor` | highest floor a part can be raised to (default 3) |
| `sync_children` | mounted modules (guns etc.) follow the part (default true); without it a gun can end up buried under a raised block |
| `child_offset` | floor of a mounted module relative to its mount: `0` same floor, `1` one above |
| `hint` | `"native"` draws the readout in the game with its own font; `"window"` uses a separate transparent window (try it if the native text does not show); `"off"` shows nothing, the keys still work |
| `keys.raise` / `lower` / `auto` | a single letter or digit, `F1`..`F12`, or a name: `Backslash`, `Slash`, `Comma`, `Period`, `Minus`, `Equals`, `Semicolon`, `Quote`, `Backquote`, `LeftBracket`, `RightBracket`, `Tab`, `Space`, `PageUp`, `PageDown`, `End`, `Home`, `Insert`, `Delete` (not case sensitive). An unknown name is logged and that key keeps its default |

If you used the old standalone `elevation.dll`, remove it: this section replaces it and `elevation.ini` is no longer read.

## Every key (`qol.full.json`)
`Modloader/config/qol.full.json` in the pack is a complete `qol.json` of this build: all of logdot's keys as on his `main` (which has `refuel` and `speed`) plus our `elevation`, with the default values. It is not read by the game; copy it over `qol.json` (or take single keys from it). The `planes` block is the game's own aircraft table as the mod writes it by default. The only crash fix in the mod is **`enable_flare_crash_fix`** (a proximity-fused missile meeting a flare whose link is gone), **on by default; keep it on, `hf_ai` needs it**. The other `enable_*` keys are gameplay tweaks, all off except the zoom.

| Key | Default | What it does |
|---|---|---|
| `enable_anti_wobble` | `false` | Stops mod-added combat interface elements from moving with screen shake. Disabled by default. |
| `enable_unblocked_guns` | `false` | Allows guns to fire through parts of the ship carrying them. Disabled by default so the normal gun-blocking rules remain active. |
| `enable_reduced_shake` | `false` | Greatly reduces combat screen shake from gunfire, thrusters, and similar effects. Disabled by default. |
| `enable_flare_crash_fix` | `true` | Prevents a crash in version 1.163 when a missile fuze loses its linked target. Enabled by default. Version 1.151 does not require this fix. |
| `enable_unblocked_ttl` | `false` | Stops armor-piercing projectiles from being forced to a shorter lifetime. Disabled by default. |
| `enable_arcade_zoom` | `true` | Enables configurable zoom levels during combat. Enabled by default. |
| `max_zoom_level` | `5` | Highest zoom level the player can select. This level must have a matching entry in `zoom_levels`. The default is `5`. |
| `min_zoom_level` | `3` | Lowest zoom level the player can select and the level used when combat begins. The default is `3`. |
| `zoom_levels` | `[14.0, 7.0, 1.0, 0.7, 0.5, 0.3]` | Zoom value for each level, starting with level `0`. The list must contain every level from `0` through `max_zoom_level`. |
| `planes` | `(game table, see qol.full.json)` | Settings and loadouts for each aircraft type. Each key is an aircraft's internal name. The default configuration copies the aircraft and loadout names from the game. The standard `30.0` health is automatically assigned. |
| `enable_shop_parts` | `false` | Adds the entries in `shop_parts` to city shops. Disabled by default. |
| `shop_parts` | `{}` | Additional parts that can appear in city shops, grouped by internal part name. A part can have one rule or several rules. Multiple rules are useful when it should have different chances or quantities in different city types. |
| `sell_multiplier` | `1.0` | Multiplies the money received when selling ship parts. `1.0` gives the normal amount, `2.0` doubles it, and `0.5` halves it. |
| `refuel` | `{"player_rate_multiplier": 1.0, "ai_rate_multiplier": 1.0}` | Changes how fast fleets refuel at stations. Only has an effect on Steam 1.163. Uses the game's own rate by default. |
| `speed` | `{"preset": "vanilla", "coefficient": 50.0, "subtract": 1....` | Changes the map cruise-speed formula. Only has an effect on Steam 1.163. Uses the game's own curve by default. |
| `elevation` | `{"enabled": false, "max_floor": 3, "sync_children": true,...` | Lets you raise and lower the elevation of the part held in the ship editor. Only has an effect on Steam 1.163. Disabled by default. |

`planes` (per aircraft: health, `loadouts` with `vec_parts`, `launch_loadout_weight`, `gun_ammo`) and `shop_parts` (extra parts in shops: `probability`, `min_parts`, `max_parts`, `city_types`) are logdot's; see his README. Do not turn on QoL's own `ai_flares` or missile-dump keys if you find them in an older fork file: `hf_ai` carries both and warns in its log if both are on.
