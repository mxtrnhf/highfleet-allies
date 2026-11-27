HIGHFLEET ALLIES — PACK v@PACK_VERSION@
Allies @ALLY_VERSION@ + Combat AI @AI_VERSION@ + ACES @ACES_VERSION@ + CCU @CCU_VERSION@
Optional bundled QoL @QOL_VERSION@
HighFleet Steam 1.163 only
Source: https://github.com/mxtrnhf/highfleet-allies

INSTALL
1. Close the game and back up Saves and Modloader/config/.
2. Install logdot's Modloader v4.2.0-preview.1:
   https://github.com/logdot/Highfleet-Modloader/releases/tag/v4.2.0-preview.1
3. Extract this ZIP into the folder containing Highfleet.exe. Merge Modloader/
   and Media/. Keep only the DLLs you want, one version of each. Remove the
   pre-split ally DLL before using ACES or CCU.
4. Merge your custom settings into ally.json, hf_ai.json, and qol.json. The
   included qol.json is active as shipped. Move crash_button into aces.json
   if you use it. Set enable to false in ccu.json if you don't want shelling.
5. Keep the QoL flare crash fix enabled to use Combat AI's automatic flares.

CONTENTS
- Modloader/mods/: hf_ally_v@ALLY_VERSION@.dll, hf_ai_v@AI_VERSION@.dll,
  hf_aces_v@ACES_VERSION@.dll, hf_ccu_v@CCU_VERSION@.dll, hf_qol_v@QOL_VERSION@.dll
- Modloader/config/: ally.json, hf_ai.json, aces.json, ccu.json, qol.json, logging.json.
- Allies_Manual_V2.pdf: updated player manual. Co-op is optional and separate.
- NOTES.md and docs/: release notes and settings references.
- Media/Snd/RADIO/radio_stall_01.wav: custom Russian voice line.
- manifest.json: source commit, component versions, validation and file hashes.
- Built without optional Cargo features.

CREDITS
Built on logdot's Highfleet-Modloader, patchy-rs, highfleet-rs and highfleet-qol.
MIT licenses included. Unofficial fan project, not affiliated with HighFleet's
creators or publisher. No base-game executable is included.
