# HighFleet Allies v@PACK_VERSION@

For **HighFleet Steam 1.163**.

## Changes

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

## Validation

Windows cross-builds passed for ally, AI and QoL. This update has not been validated in-game; the classic/KAZ options still need gameplay testing.
