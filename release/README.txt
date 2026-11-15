HIGHFLEET ALLIES — PACK v@PACK_VERSION@
hf_ally @ALLY_VERSION@ + hf_ai @AI_VERSION@ + bundled QoL — Steam 1.163 only
Source: https://github.com/mxtrnhf/highfleet-allies

WHAT CHANGED
- Cooperative dodge boosts (hf_ai):
  - Previously, tactical dashes blindly overwrote engine angles and fought
    native obstacle avoidance/gravity hold. Now angle overrides only apply when
    native thrust agrees within 60 degrees.
- Hull-size-aware dive range & keep-out barriers (hf_ai):
  - Previously, center-based spacing caused ships to misjudge big hulls or
    yield too early in fast dogfights. Now dive_range and keep-out barriers
    (120-280u surface-to-surface) use actual hull radii across nearby ships,
    and emergency yield only fires on imminent impact (<70u or <0.45s TTC).
- Hostile-only dive denial (hf_ai):
  - Previously, friendlies passing underneath could trigger dive denial. Now
    only airborne enemies do.
- Marginal-TWR lateral dodge floor (hf_ai):
  - Previously, ships just above twr_cutoff (2.0, e.g. Wasp) stalled during
    AI DIVE. Now downward level-pull still ramps 0-100% from twr_cutoff to
    TWR 4.0, while lateral sidesteps keep a 40% floor.
- Any-border ally retreat & GTFO steering (hf_ally + hf_ai):
  - Previously, retreating allies only exited at the top, got blocked by the
    300u edge margin, or got stuck around enemies running combat tactics. Now
    retreating AI allies pick the nearest unblocked border (left, right, or
    top), route around nearby hulls, ignore the edge margin, use a gentle
    1.25x cruise boost, and extract at any border.
- Second-player co-op compatibility (hf_ally, issue #1).
- Note: Allies_Manual.pdf is still from an earlier version pending testing.

RETAINED FROM PREVIOUS PACKS
- Shipworks repeated-FIGHT crash & growing enemy fleet fixes (hf_ally v1.3.3).
- Bundled QoL: missing missile/flare target fix, defensive null-fleet/vector
  guards, and the separate aircraft target-choice out-of-bounds fix.
- Optional hf_ai high-TWR boost limiter: twr_limit 5, twr_deceleration 0.75.
- Shipworks custom ally/enemy rosters: ADD, ADD ENEMY, CLEAR, FIGHT.

INSTALL
1. Back up Saves.
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
