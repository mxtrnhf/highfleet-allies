# HighFleet Allies v@PACK_VERSION@

## High-TWR AI ships

- **`twr_limit` and `twr_deceleration` tame ships with high Shipworks TWR,
  like Lightning, and put a bit of brakes on them.** In theory this should
  reduce the collisions they started to experience thanks to their new
  exceptional piloting skills.
- Defaults: `twr_cutoff: 2`, `twr_limit: 5`, `twr_deceleration: 0.75`.
  AI ships strictly above TWR 5 use 75% final engine strength, boosts included.
  Player control is unaffected.
- **To disable:** set `twr_limit` or `twr_deceleration` to `0`.
- This is a temporary mitigation, not collision avoidance or a direct speed
  cap; braking thrust is also reduced. In-game collision validation is pending.
- No AI tick skipping is included. Remove the obsolete `ally_tick_skip`
  (or `ai_tick_skip`) key from old experimental configs; it has no effect.

## Components

- hf_ai **v@AI_VERSION@**, hf_ally **v@ALLY_VERSION@** and bundled QoL.
- Includes the earlier Shipworks custom enemy formation and repeated-FIGHT
  crash fix. No ally gameplay changes in this pack.

## Install

For HighFleet **Steam 1.163** with Modloader v4. Back up `Saves`, extract the ZIP into the game directory, and remove older ally/AI DLLs so only one of each remains. Merge the included configs with your custom settings.
