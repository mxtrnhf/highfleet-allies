HIGHFLEET ALLIES — PACK v@PACK_VERSION@
hf_ally @ALLY_VERSION@ + hf_ai @AI_VERSION@ + bundled QoL — Steam 1.163 only
Source: https://github.com/mxtrnhf/highfleet-allies

WHAT CHANGED
- Shipworks test allies: ADD, beside TEST in the top row, captures the edited
  ship, including unsaved changes, into the ship list.
- CLEAR, beside ADD, empties the list without leaving Shipworks or changing
  the edited ship.
- The existing FIGHT button brings the listed ships into test combat as
  friendly AI allies. Add up to eight allies.
- Loading/switching designs, RESET, RETRY and returning from test combat
  preserve the list. Leaving Shipworks clears it.
- Buttons use native Shipworks styling; ship names use the PLAYER-marker font.
- ADD/FIGHT confirmed in game. CLEAR still needs in-game verification.

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
