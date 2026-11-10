HIGHFLEET ALLIES — PACK v@PACK_VERSION@
hf_ally @ALLY_VERSION@ + hf_ai @AI_VERSION@ + bundled QoL — Steam 1.163 only
Source: https://github.com/mxtrnhf/highfleet-allies

WHAT CHANGED
- Fixed: gunpod plane loadouts could not be picked on the carrier plane
  buttons. A loadout that carries no parts of its own — a gunpod, like the
  20 mm or 100 mm cannon pods added for planes by QoL/ammo mods — was
  mistaken for the plain "guns only" choice and never offered. All of a
  craft's loadouts are now selectable, gunpods first (vanilla: the 37 mm
  default); gun choices cost no stock, and a launched gunpod plane strafes
  its target until its rounds run out, then flies home.

DEBUG OVERLAY SETTINGS
Modloader/config/ally.json -> debug_overlay:
- enable (false): show the overlay.
- circles ([250, 500, 1000]): ring radii in game units; [] hides them.
- ai_waypoint (true): show AI destinations and distances.
- prediction_marker (false): show estimated aim markers for supported cannon ammo.
- pointer_length (300): native aim-line length and bearing-marker distance in
  game units; 0 hides both.
Keep top-level enabled: true; restart after editing. ally_reticle_scale controls
ally target boxes separately. hf_ai adds its AI state to ship labels.
AP, guided shells and rockets have no aim prediction.

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
- Built without diagnostic features.

CREDITS
Built on logdot's Highfleet-Modloader, patchy-rs, highfleet-rs and highfleet-qol.
MIT licenses included. Unofficial fan project, not affiliated with HighFleet's
creators or publisher. No base-game executable is included.
