HIGHFLEET ALLIES — PACK v@PACK_VERSION@
hf_ally @ALLY_VERSION@ + hf_ai @AI_VERSION@ + bundled QoL — Steam 1.163 only
Source: https://github.com/mxtrnhf/highfleet-allies

WHAT CHANGED
- Fixed second-new-battle crashes in Shipworks, even without touching ADD,
  ADD ENEMY or CLEAR. Destroyed enemy fleets left null Links; the loader and
  pre-combat roster builder dereferenced null + 0x100. Dead Links are guarded
  and removed before use.
- Fixed enemy escadras accumulating every previous test battle's opponents.
  Our enemy-append hook incorrectly relocated a RIP-relative LEA, making the
  native clear use the wrong address. An absolute MOV restores correct clearing.
- Link compaction now unregisters/copy-constructs native Links instead of moving
  their bytes. These fixes belong to hf_ally, not HEE.
- Repeated fresh FIGHTs, custom rosters and CLEAR confirmed in game, 2026-11-15.
- See NOTES.md for the crash-site details and other retained protections.
- AI behavior and configuration defaults are unchanged.

RETAINED FROM PREVIOUS PACKS
- Bundled QoL: missing missile/flare target fix, defensive null-fleet/vector
  guards, and the separate aircraft target-choice out-of-bounds fix.
  The fleet guards do not establish that every merge crash is fixed.
- Optional hf_ai high-TWR boost limiter: twr_limit 5, twr_deceleration 0.75.
  Set either to 0 to disable. Normal thrust and player control are unaffected.
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
