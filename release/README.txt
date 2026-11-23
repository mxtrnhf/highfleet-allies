HIGHFLEET ALLIES — PACK v@PACK_VERSION@
hf_ally @ALLY_VERSION@ + hf_ai @AI_VERSION@ + hf_qol @QOL_VERSION@
HighFleet Steam 1.163 only
Source: https://github.com/mxtrnhf/highfleet-allies

INSTALL
1. Close the game and back up Saves and Modloader/config/.
2. Install logdot's Modloader v4:
   https://github.com/logdot/Highfleet-Modloader
3. Extract this ZIP into the folder containing Highfleet.exe. Merge Modloader/
   and Media/. Remove older ally and AI DLLs and duplicate QoL DLLs (including
   any older highfleet_qol.dll) so only one of each remains.
4. Merge your custom settings into ally.json, hf_ai.json, and qol.json. The
   included qol.json is active as shipped; there's no qol.full.json to rename.
5. Keep the QoL flare crash fix enabled when using hf_ai.

CONTENTS
- Modloader/mods/: hf_ally_v@ALLY_VERSION@.dll, hf_ai_v@AI_VERSION@.dll, hf_qol_v@QOL_VERSION@.dll
- Modloader/config/: ally.json, hf_ai.json, qol.json, and logging.json.
- Allies_Manual_V2.pdf: updated player manual. Co-op is optional and separate.
- NOTES.md and docs/: release notes and settings references.
- Media/Snd/RADIO/radio_stall_01.wav: custom Russian voice line.
- manifest.json: source commit, component versions, validation and file hashes.
- Built without optional Cargo features.

CREDITS
Built on logdot's Highfleet-Modloader, patchy-rs, highfleet-rs and highfleet-qol.
MIT licenses included. Unofficial fan project, not affiliated with HighFleet's
creators or publisher. No base-game executable is included.
