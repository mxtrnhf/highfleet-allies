# HighFleet Allies v@PACK_VERSION@

Includes **hf_ally v@ALLY_VERSION@**, **hf_ai v@AI_VERSION@**, **hf_qol v@QOL_VERSION@**, and the updated **Allies Manual V2** (`Allies_Manual_V2.pdf`).

## What changed since v1.3.5

- **Allies:** Press **Z** to order your allies to attack the selected enemy. Shipworks allies now respect `max_allies`, `ally_delay_seconds`, and the three-game-second spacing between deployments. Includes a Map Mode Extended compatibility fix and fixes for gunpod aircraft retargeting.
- **Combat AI:** Set separate reload rates for enemies and AI allies. Common missile evasion now requests full throttle while retaining native obstacle avoidance and gravity compensation.
- **QoL:** Optional skippable intro restoration, pre-fire line visibility for both sides, and an independent enemy warning sound toggle. The pack ships the active `Modloader/config/qol.json`; there is no `qol.full.json` to rename.

## Install

For **HighFleet Steam 1.163** with Modloader v4. Back up `Saves` and `Modloader/config`, then extract the release ZIP into the folder containing `Highfleet.exe`. Remove older ally, AI, and QoL DLLs (including any older `highfleet_qol.dll`) so only one of each remains. Merge your custom settings into `ally.json`, `hf_ai.json`, and `qol.json`. Keep the QoL flare crash fix enabled when using `hf_ai`.
