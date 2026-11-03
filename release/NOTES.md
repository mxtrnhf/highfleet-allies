# HighFleet Allies v@PACK_VERSION@

For **HighFleet Steam 1.163**.

## Changes since v0.14.5

- **New: combat ship switching** (config **ship_switching**, default **on**). **TAB** cycles the deployed friendly ships, **1** takes the ship shown in the top ally panel and **2** the ship in the bottom panel. The selected ship becomes player-controlled on the next combat tick; your former ship drops to the game's native AI (driven by hf_ai when installed), keeps ally tracking and takes over the freed panel slot. Held gun/flare/boost input moves to the new ship, and retreat keys plus the low-fuel retreat rule then apply to it. Retreating, destroyed, inactive and non-flying ships are never selected, and the keys are ignored during pre-combat, pause, dialogs or when another window has focus. Set `ship_switching: false` to free the keys for other mods. Details: docs/ship-switching.md.
- **Improved ally ship panels.** The panel now draws the complete hull with the game's own palette, layer and draw order, so an ally panel looks like an enemy panel instead of a dark silhouette with only the modules visible. The panel had been relying on a marker table the game rebuilds every frame, which never contains a mod-deployed ship; it now writes the same per-part records itself for the duration of its own render, using the game's layer and scale rules.
- **Ally panels no longer stop rendering an off-screen ally.** The schematic is independent of the camera and keeps updating while the ally is outside the view, and an ally that leaves the roster (destroyed, retreated, handover) loses its panel instead of leaving a stale picture. Panels still disappear with the stock panels at the end of a battle.
- **Ally target boxes** are drawn at 80% of the native square so several stay readable next to your own box, and the size is configurable with **ally_reticle_scale** (default `0.8`, `1.0` = native size).
- **Fixed an access violation after a battle or scene change** in the retained pre-combat ALLY / plane buttons: cached button pointers are validated against the current native GUI registry before their names or fields are read, and a damaged button sprite is repaired in place instead of appending another background.
- Split panel bookkeeping so a handoff or a retreat cannot leave one ally's schematic in another ally's panel slot.

## Pack contents

hf_ally @ALLY_VERSION@, hf_ai @AI_VERSION@ (unchanged since v0.14.5), and the bundled QoL build (unchanged).

## Installation

Back up your saves and configs. Extract the pack into the folder containing Highfleet.exe, keeping only one ally DLL and one AI DLL. The bundled ally/AI configs replace existing files if extracted over them; merge your custom values instead if needed.

The QoL build is optional. Keep its flare crash fix enabled. Its full JSON file is a reference, not an active config.
