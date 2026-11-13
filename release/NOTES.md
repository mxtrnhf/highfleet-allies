# HighFleet Allies v@PACK_VERSION@

## Shipworks test allies and enemies

- **Added ability to construct custom enemy formation for shipworks battle.** **ADD ENEMY**, right of ADD in the top row, captures the currently edited ship into an enemy list (red text). With a non-empty enemy list, the existing **FIGHT** button overrides the default enemy generation and fields exactly those ships as enemies—up to **eight**.
- **ADD**, beside TEST in the top row, captures the currently edited ship—including unsaved changes—into the ally list. **FIGHT** brings the listed ships into test combat as friendly AI allies. Add up to **eight allies**.
- **CLEAR**, beside ADD, empties both lists without leaving Shipworks or changing the edited ship. Loading or switching designs, RESET, RETRY, and returning from test combat preserve the lists. Leaving Shipworks clears them.
- A custom FIGHT's enemy fleet is pruned again afterwards, so cleared enemies never leak into later battles.
- Buttons match the native Shipworks font/style; ship names use the mod's PLAYER-marker font.

ADD ENEMY/FIGHT/CLEAR confirmed working in game by the maintainer.

## Install

For HighFleet **Steam 1.163** with Modloader v4. Back up `Saves`, extract the ZIP into the game directory, and remove older ally/AI DLLs so only one of each remains. Merge the included configs with your custom settings.
