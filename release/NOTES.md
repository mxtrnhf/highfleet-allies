# HighFleet Allies v@PACK_VERSION@

## Shipworks: repeated battles without crashes or growing enemy fleets

1. **Null-reference crashes:** destroyed Shipworks enemy fleets left empty Links
   in the battle list. Both the Combat loader and the later pre-combat roster
   builder could dereference them, crashing at `0x1402c3a04` or `0x1402d9806`
   while reading `null + 0x100`. Dead Links are now skipped/removed before use,
   including when ADD, ADD ENEMY and CLEAR were never touched.
2. **Growing escadras:** starting another test battle could retain every previous
   enemy fleet. The normal clear-and-rebuild sequence is restored, so each FIGHT
   uses a fresh enemy formation instead of accumulating old opponents.
3. **What went wrong:** our enemy-append hook moved a RIP-relative
   `LEA RCX,[enemy_list]` into a trampoline but copied its bytes without adjusting
   the displacement. The native clear therefore received the wrong address.
   It now uses an absolute `MOV RCX,imm64`. Link compaction also uses native
   unregister/copy operations, not raw byte moves, preserving Link registrations.

The earlier null checks prevented the immediate crash but did not fix the
underlying clear-address error. This release includes both corrections.
**These Shipworks bugs were in hf_ally, not HEE.**

### Other retained crash protections

- **Lost missile/flare target:** bundled QoL retains the flare/proximity-fuze
  crash fix for a missing linked target.
- **Missing fleets:** bundled QoL guards a null fleet argument and two null
  fleet-vector elements, taking the native "no ship" path. These are defensive
  guards, **not a claim that every merge crash is fixed**.
- The separate stock aircraft-AI fix covers an out-of-bounds target-choice read;
  that is not the same bug as the null Shipworks fleet Links.

### Validation and components

Repeated fresh FIGHTs, custom ally/enemy rosters and CLEAR were confirmed working
in game on Steam 1.163 (2026-11-15). The debug log shows the enemy fleet list
staying at one previous fleet before replacement, rather than growing to six;
the original second-sortie crash did not recur.

Includes **hf_ally v@ALLY_VERSION@**, **hf_ai v@AI_VERSION@** and bundled QoL.
AI behavior, HEE and configuration defaults are unchanged from the previous pack.

## Install

For HighFleet **Steam 1.163** with Modloader v4. Back up `Saves`, extract the ZIP
into the game directory, and remove older ally/AI DLLs so only one of each remains.
Use only one QoL build: do not stack bundled `highfleet_qol.dll` with another QoL
fork. Merge the included configs with your custom settings.
