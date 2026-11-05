# HighFleet Allies v@PACK_VERSION@

For **HighFleet Steam 1.163**.

> ## Minor update
> This is a small ally-side follow-up to v0.14.17: pre-combat button click sounds and an optional Zenith hold-fire for AI allies. `hf_ai` and QoL are unchanged.
>
> ## Config compatibility
> **Updating from v0.14.17:** merge the new `Modloader/config/ally.json` from this pack (or just add `"ally_zenith_autofire": true`). The pack's ally config is updated for this release: it carries the new Zenith key and keeps the existing button settings (the click sounds need no key of their own). An older config without the key keeps the vanilla default `true`. The `hf_ai.json` preset is unchanged since hf_ai v0.10.11.
>
> **Updating from v0.14.16 or older:** no separate `hf_ai.json` change is needed unless you come from v0.14.13 or older, where the hf_ai preset still has old dive/flare values: replace `Modloader/config/hf_ai.json` with the one in this pack (or delete it so the mod generates a fresh preset).

## Changes since v0.14.17

### `hf_ally v@ALLY_VERSION@` — pre-combat button click sounds
- Clicking one of the pre-combat ALLY ship on/off buttons or a carrier plane-loadout button now plays the stock `Media/Snd/button_01.wav` click.
- The sound plays only when a click actually changes the choice. Every other pre-combat control stays silent.
- It uses the existing `ally_toggle` and `plane_loadouts` buttons and is independent of `ally_sounds`; no new config key is needed.

### `hf_ally v@ALLY_VERSION@` — optional ally Zenith hold-fire
- New `ally_zenith_autofire` in `ally.json` (default `true`, vanilla behaviour). Set it to `false` and AI-run allies no longer launch their own ship missiles (the native Zenith launch path).
- The ship you fly keeps manual launches. Enemies, neutrals and player-flown ships are unaffected.
- The guard sits on the native launch entry, so it also covers `hf_ai`'s missile-burst wrapper. Ship-bomber missile suppression shares the same guard.

### Updated ally config
- The pack's `Modloader/config/ally.json` gains the new `ally_zenith_autofire` key. Merge it into your existing config; an old config still loads and uses `true` for the missing key.
- The button click sounds need no key: they follow the existing ALLY and plane-loadout buttons.
- `Modloader/config/hf_ai.json` is unchanged.

### Validation
- GitHub Actions builds all three Windows DLLs without diagnostic features.
- The installed hf_ally v@ALLY_VERSION@ build was exercised by the maintainer; no automated regression tests were added.
- In-game validation of the full release pack remains the player's responsibility.

## Pack contents

`hf_ally v@ALLY_VERSION@`, `hf_ai v@AI_VERSION@`, and the bundled QoL build.

## Installation

1. Back up your `Saves` folder.
2. Extract the pack into the folder containing `Highfleet.exe`.
3. Remove any older `hf_ai_v*.dll` and `hf_ally_v*.dll` files from `Modloader/mods/` so only one of each DLL is present.
4. Merge the new `Modloader/config/ally.json` key into your existing config if you keep a custom profile.
