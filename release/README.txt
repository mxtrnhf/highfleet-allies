HIGHFLEET ALLIES — PACK v@PACK_VERSION@
hf_ally @ALLY_VERSION@ + hf_ai @AI_VERSION@ + optional QoL build — Steam 1.163 only
Source: https://github.com/mxtrnhf/highfleet-allies
Research notes: https://github.com/mxtrnhf/hf_notes

===============================================================================
MINOR UPDATE — WHAT CHANGED SINCE PACK v0.14.17
===============================================================================
This is a small ally-side follow-up. hf_ai and QoL are unchanged.

- hf_ally @ALLY_VERSION@: pre-combat button click sounds. Clicking an ALLY
  ship on/off button or a carrier plane-loadout button plays the stock
  button_01 click when the choice actually changes. No new config key.
- hf_ally @ALLY_VERSION@: optional ally Zenith hold-fire. New
  ally_zenith_autofire key (default true = vanilla). Set false and AI-run
  allies stop launching their own ship missiles; the ship you fly can still
  launch manually. Enemies and player-flown ships are unaffected, and
  hf_ai's missile-burst wrapper is covered.
- Updated ally config: Modloader/config/ally.json gains ally_zenith_autofire.
  Merge it into your existing config. Old configs still load and use true for
  the missing key. Button sounds need no key and follow the existing buttons.
  hf_ai.json is unchanged.

===============================================================================
CONFIG COMPATIBILITY
===============================================================================
Updating from v0.14.17: merge the new Modloader/config/ally.json (or just add
"ally_zenith_autofire": true). The hf_ai.json preset is unchanged since
hf_ai v0.10.11.

Updating from v0.14.16 or older: no separate hf_ai.json change is needed unless
you come from v0.14.13 or older, where the hf_ai preset still has old dive and
flare values. In that case overwrite Modloader/config/hf_ai.json with the new
file from this ZIP (or delete your old hf_ai.json so the mod generates a fresh
one). The v0.10.11 preset trims hf_ai.json from 80 to 49 total leaf settings
(48 gameplay settings plus logging.log_panic), reworks universal aim dodging and
early fire, merges dive settings, and updates default ranges/boosts
(launch_flares_evade_range=400, dive_dwell_s=0.5, dive_boost=2.0,
aim_boost=2.0). Keeping an old hf_ai.json will feed stale values into the AI.
===============================================================================

INSTALL
1. Back up Saves.
2. Install logdot's Modloader v4:
   https://github.com/logdot/Highfleet-Modloader
3. Extract this ZIP directly into the folder containing Highfleet.exe. Merge
   Modloader/ and Media/, and remove previous hf_ally_v*.dll / hf_ai_v*.dll
   files from Modloader/mods/ so only ONE of each is present.
4. Merge the new ally_zenith_autofire key into your existing ally.json if you
   keep a custom profile.
5. Keep the QoL flare crash fix enabled. The optional highfleet_qol.dll is our
   logdot QoL main + manual editor elevation and firing-arc rules build.

FILES AND VALIDATION
- Modloader/mods/hf_ally_v@ALLY_VERSION@.dll, hf_ai_v@AI_VERSION@.dll, highfleet_qol.dll
- Modloader/config/ally.json, hf_ai.json, logging.json, qol.full.json
- Media/Snd/RADIO/radio_stall_01.wav: our voice line, replacing that game file
- docs/: player-defaults, ally settings, debug overlay, AI settings/tuning and QoL guide
- manifest.json: source commit, component versions and per-file SHA256 hashes
- Built by GitHub Actions without diagnostic features. The installed hf_ally
  build was exercised by the maintainer; no automated regression tests were added.
  In-game validation of the full pack remains the player's responsibility.

CREDITS
Built on logdot's Highfleet-Modloader, patchy-rs, highfleet-rs and highfleet-qol.
MIT licenses are included. Unofficial fan project; not affiliated with HighFleet's
creators or publisher. No base-game executable is included.
