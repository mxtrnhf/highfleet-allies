# QoL settings: refuel, speed and elevation (`Modloader/config/qol.json`)

The `highfleet_qol.dll` in the release pack is [logdot's highfleet-qol](https://github.com/logdot/highfleet-qol) `main` plus our manual elevation (source in [`../qol`](../qol)). **logdot's stock QoL release (v1.3.0) works fine with our mods; it just does not have the three sections below.** They are included until the changes are merged into a release of the main repo, then use his release.

| Section | What it does | Where it comes from |
|---|---|---|
| `refuel` | how fast fleets refuel at stations | logdot's `main` (our PR #19, merged, no release yet) |
| `speed` | the map cruise-speed formula | logdot's `main` (our PR #19) |
| `elevation` | raise/lower the floor of a held part in the ship editor | **our PR [#20](https://github.com/logdot/highfleet-qol/pull/20), still a draft** |

## Getting the settings into your file
* **No `qol.json` yet:** start the game once with the DLL; it writes a full `qol.json` that already has all three sections (with vanilla values, elevation off).
* **You already have an older `qol.json`:** the DLL does **not** rewrite it. Open `Modloader/config/qol.example.json` from the pack, copy the three sections (`refuel`, `speed`, `elevation`) into your `qol.json` next to the other top-level keys, and edit them. Missing sections simply take the defaults, so nothing breaks if you skip them.
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

## Other settings in the same file
`enable_flare_crash_fix` (default **on**: keep it on, `hf_ai` needs it), `enable_arcade_zoom`, `min_zoom_level` / `max_zoom_level` / `zoom_levels`, `enable_anti_wobble`, `enable_unblocked_guns`, `enable_reduced_shake`, `enable_unblocked_ttl`, `planes` (aircraft health and loadouts, written from the game's tables), `enable_shop_parts` / `shop_parts`, `sell_multiplier`. These are logdot's; see his README. Do not turn on QoL's own `ai_flares` or missile-dump keys if you find them in an older file: `hf_ai` carries both, and `hf_ai` logs a warning if both are on.
