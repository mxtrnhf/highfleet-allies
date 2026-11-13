HIGHFLEET ALLIES — PACK v@PACK_VERSION@
hf_ally @ALLY_VERSION@ + hf_ai @AI_VERSION@ + bundled QoL — Steam 1.163 only
Source: https://github.com/mxtrnhf/highfleet-allies

WHAT CHANGED
- twr_limit and twr_deceleration tame ships with high Shipworks TWR, like
  Lightning, and put a bit of brakes on them. In theory this should reduce
  the collisions caused by their new exceptional piloting skills.
- Defaults: twr_cutoff: 2, twr_limit: 5, twr_deceleration: 0.75.
  Above TWR 5, only hf_ai aim/dive boosts, panic boosts and boosted barrel
  rolls are multiplied by 0.75: boost 2 becomes 1.5. Normal engine strength,
  vanilla avoidance, boost settings of 1 and player control are unaffected.
  To disable, set twr_limit or
  twr_deceleration to 0.
- Replaces withdrawn v1.3.2, which incorrectly reduced all engine strength.
- Temporary mitigation, not a direct speed cap; stock braking is unchanged.
  In-game collision validation is pending.
- No AI tick skipping. Remove obsolete ally_tick_skip / ai_tick_skip keys
  from experimental configs; they have no effect.

RETAINED FROM PREVIOUS PACKS
- Fixed a bug that crashed the game on a repeated Shipworks test FIGHT:
  a finished test battle's destroyed enemy fleet left a null-pointer node
  in the enemy fleet list, and the combat level loader dereferenced it
  unchecked on the next FIGHT (access violation at 0x1402c3a04). The enemy
  list is now cleared before each custom FIGHT and null nodes are dropped
  and guarded.
- Added ability to construct custom enemy formation for shipworks battle:
  ADD ENEMY, right of ADD, captures the edited ship into an enemy list (red
  text); with a non-empty list, FIGHT overrides the default enemy generation
  and fields exactly those ships as enemies. Up to eight enemies.
- Shipworks test allies: ADD, beside TEST in the top row, captures the edited
  ship, including unsaved changes, into the ship list; FIGHT brings them into
  test combat as friendly AI allies. Add up to eight allies.
- CLEAR, beside ADD, empties both lists without leaving Shipworks or changing
  the edited ship. Loading/switching designs, RESET, RETRY and returning from
  test combat preserve the lists. Leaving Shipworks clears them.
- A custom FIGHT's enemy fleet is pruned again afterwards, so cleared enemies
  never leak into later battles.
- Buttons use native Shipworks styling; ship names use the PLAYER-marker font.
- ADD ENEMY/FIGHT/CLEAR confirmed in game.

INSTALL
1. Back up Saves. Read Allies_Manual.pdf.
2. Install logdot's Modloader v4:
   https://github.com/logdot/Highfleet-Modloader
3. Extract this ZIP into the folder containing Highfleet.exe. Merge Modloader/
   and Media/. Remove older hf_ally_v*.dll / hf_ai_v*.dll and duplicate QoL DLLs
   from Modloader/mods/ so only one of each remains.
4. Merge the included ally.json and hf_ai.json with your custom settings.
5. Keep the QoL flare crash fix enabled.

CONTENTS
- Modloader/mods/hf_ally_v@ALLY_VERSION@.dll, hf_ai_v@AI_VERSION@.dll, highfleet_qol.dll
- Modloader/config/: player presets and optional QoL settings.
- Allies_Manual.pdf, docs/: player manual and configuration references.
- Media/Snd/RADIO/radio_stall_01.wav: custom Russian voice line.
- manifest.json: source commit, component versions, validation and file metadata.
- Built without optional Cargo features.

CREDITS
Built on logdot's Highfleet-Modloader, patchy-rs, highfleet-rs and highfleet-qol.
MIT licenses included. Unofficial fan project, not affiliated with HighFleet's
creators or publisher. No base-game executable is included.
