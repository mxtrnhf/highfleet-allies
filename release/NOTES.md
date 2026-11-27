# HighFleet Allies v@PACK_VERSION@

**Experimental prerelease.** See `manifest.json` for the build's validation.
Back up your saves and configuration before trying it.

Includes **Allies v@ALLY_VERSION@**, **Combat AI v@AI_VERSION@**, **ACES v@ACES_VERSION@**,
**CCU v@CCU_VERSION@**, optional **QoL v@QOL_VERSION@**, and the **Allies Manual V2**.

Shipworks rosters and A.C.E. now live in ACES. Campaign shelling now lives in
CCU. The four mods can be selected individually and use Modloader's shared
Patchy service.

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

For **HighFleet Steam 1.163** with **Modloader v4.2.0-preview.1**. Back up `Saves`
and `Modloader/config`, then extract the ZIP beside `Highfleet.exe`. Keep one
version of each mod you want. Remove the pre-split Allies DLL before enabling
ACES or CCU. Move `crash_button` into `aces.json` if you use it. The QoL flare
crash fix is required for Combat AI's automatic flares.
