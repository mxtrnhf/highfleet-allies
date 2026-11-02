# HighFleet Allies v@PACK_VERSION@

For **HighFleet Steam 1.163**.

## Changes

- Added **ship switching** to ally (config **ship_switching**, default **on**): press **1** or **2** to take the ship actually shown in the top or bottom ally panel, or **TAB** to cycle deployed friendly ships. The selected ship becomes player-controlled at the next combat tick; your former ship drops to the game's native AI (driven by hf_ai when installed), keeps ally tracking and takes over the freed panel slot. Held gun/flare/boost input moves to the new ship. Retreat keys and low-fuel retreat then apply to the new ship. Retreating, destroyed, inactive and non-flying ships are never selected; keys are ignored during pre-combat, pause, dialogs or when another window has focus. Set `ship_switching: false` to free the keys for other mods.
- Fixed the ally ship panels: the schematic now runs the same native HUD marker initialisation as a stock enemy ship (`Body::UpdateHudMarkers` with the initialisation flag), so an ally is drawn fully even when it is off-screen, with the normal palette and brightness. The panels still clear when the battle ends.

- Added **ally_panels** to ally (default **on**): draws a half-size copy of the battle ship panel for each ally beside your own bottom-left panel, showing that ally's name, speed, counters and live module schematic. Two fit in the column; they appear only while an ally is in the battle.
- Added **ally_reticles** to ally (default **on**): draws the game's native target box around the enemy each ally is currently targeting, in the ally colour (`ally_damage_color`). Your own lime target box is unchanged.
- Added **ally_bombers** to ally (default **off**): bomb-carrying wingmen now perform a player-like bombing run against landed, non-prize enemies. The dive is governed so the ship can pull out, aiming uses the game's own ballistic prediction, the whole payload is released at 0.05 s intervals below 1000 units with real downward speed, flares fire every 0.5 s while armed, ship-missile launches are suppressed during the run, and the ship climbs away afterwards. See docs/ship-bombers.md.
- Adjusted default `launch_flares_evade_range` to **700.0** to stop AI from overusing the afterburner.
- Fixed a crash in diagnostic logging after restarting a battle. Cached ally/enemy probes are reset and validated before reading ship data.
- Moved the stock aircraft AI crash fix and null-fleet/vector crash guards from ally into bundled QoL. Both remain enabled by default, now controlled by **fix_plane_ai_crash** and **fix_merge_crash** in **qol.json**. The fleet guards are defensive, not a confirmed fix for every merge crash. Update both DLLs together; stock upstream QoL does not include these additions.
- Removed ship/battle CSV recording, steering/obstacle CSV capture, capture markers and replay-analysis tools.
- Kept megalog, crash reports and other text diagnostics.
- Grouped logging options under **logging** in ally and AI configs. Old logging-key locations remain supported; explicit nested settings take precedence.
- Retained the optional dead-node crash diagnostic as **logging.dead_node**, disabled by default.
- Added **elevation_rules** to the bundled QoL build, ported from our separate QoL fork:
  - **vanilla** (default) and **modern** leave Steam 1.163 firing-arc rules unchanged.
  - **classic** restores the older same-floor gun-blocking and KAZ target-cone/coverage rules.
  - **classic_kaz** adds a KAZ truss exemption to classic rules; hull blocks, hardpoints, armour and modules still block it.
- Firing-arc rules are independent of manual ship-editor elevation. Existing **qol.json** files are not overwritten.

## Installation

Back up your saves and configs. Extract the pack into the folder containing Highfleet.exe, keeping only one ally DLL and one AI DLL. The bundled ally/AI configs replace existing files if extracted over them; merge your custom values instead if needed.

The QoL build is optional. Keep its flare crash fix enabled. Its full JSON file is a reference, not an active config.
