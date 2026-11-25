# HighFleet Allies v@PACK_VERSION@

**Experimental prerelease — not stable.** The maintainer has validated the
changes in game, but features and balance may still change. Back up your saves
and configuration before trying it.

Includes **hf_ally v@ALLY_VERSION@**, **hf_ai v@AI_VERSION@**, **hf_qol v@QOL_VERSION@**, and the updated **Allies Manual V2** (`Allies_Manual_V2.pdf`).

## What changed since v1.3.6

- **Shipworks A.C.E. (Advanced Custom Encounters):** Try Missile Defense, Airstrike, and Silent Strike from the new menu. The mod's Shipworks toolbar can be collapsed without clearing the rosters.
- **Missile Defense:** Select conventional or nuclear strategic missiles from a scrollable list, and deploy defenders on the ground or airborne. Anti-radiation missiles target active radar or jammers. Damage View shows defender module panels; Preserve Damage carries surviving ships and damage into later launches on the same roster. Nuclear effects and damage have been reworked toward campaign behavior, but balance and fidelity remain under investigation.
- **Airstrike:** Configure aircraft groups, counts and loadouts from the game's live tables, choose ground or airborne defenders, and launch against a friendly or enemy-only roster. Damage View and Preserve Damage support replaying successive raids. Mixed friendly and enemy rosters are not supported in this mode.
- **Silent Strike:** Start a test battle through the usual pre-combat screen with enemies deployed landed.
- **Shipworks usability:** Identical same-name roster snapshots are grouped with counts; different refits remain separate. Missile and aircraft-loadout pickers have scrolling and selected-row highlights.
- **Diagnostics and compatibility:** Improved hang diagnostics and hook lock handling; QoL pre-fire warnings now use patchy source validation.

Nuclear damage may differ from campaign results and remains under investigation.
Please report reproducible problems with logs.

## Install

For **HighFleet Steam 1.163** with Modloader v4. Back up `Saves` and `Modloader/config`, then extract the release ZIP into the folder containing `Highfleet.exe`. Remove older ally, AI, and QoL DLLs (including any older `highfleet_qol.dll`) so only one of each remains. Merge your custom settings into `ally.json`, `hf_ai.json`, and `qol.json`. Keep the QoL flare crash fix enabled when using `hf_ai`.
