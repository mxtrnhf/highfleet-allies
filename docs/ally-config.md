# hf_ally settings (`Modloader/config/ally.json`)

Every key below is already in the `ally.json` of the release pack, so you do not have to search for flags. A missing key takes the default shown. Edit the file while the game is closed. `"enabled": false` turns the whole mod off.

The checked-in [player preset](../ally/config/ally.json), compiled defaults, and
release config now use the same settings (hf_ally 0.14.2). `logging.debug_log` is `true`
in this preset, so detailed logs/watchdog/crash dumps are active; diagnostic mode
can force probe/pre-combat logging even when those individual keys are false.
Ship/battle CSV capture and F9 capture markers have been removed.
The preset's numeric colour `58623` is equivalent to `"#00E4FF"`; newly generated
configs write the HEX spelling.

## Missing keys and how to revert to vanilla

**You do not have to list every key.** Any key may be omitted and hf_ally supplies the code default shown below. A short file containing only the settings you want to change is valid; an empty `{}` is valid too. If the file is absent, hf_ally tries to create it with the full defaults; if the JSON is invalid, it logs an error and uses the defaults.

**Player default does not mean vanilla.** The compiled defaults and release preset both enable mod features and plane tuning. Use the explicit values below for stock behaviour. There is no general `-1` disable convention.

For the most complete revert, use only:

```json
{ "enabled": false }
```

For individual features, use this table. Settings for an already-disabled parent feature can be omitted.

| Feature | Setting that disables it or restores vanilla | What remains / important dependency |
|---|---|---|
| Entire mod | `"enabled": false` | Closest complete config revert; startup/crash-reporting infrastructure can still load with the DLL. |
| Ally deployment | `spawn_ally: false` | No mod-deployed wingmen. Independent enemy, plane, damage-colour, intercept and crash-fix features remain. |
| Ally arrival delay only | `ally_delay_seconds: 0.0` | Allies still deploy immediately; use `spawn_ally: false` to deploy none. |
| Number of allies | **No zero/disable value** | `max_allies: 0` is clamped to one. Use `spawn_ally: false` for none. |
| Pre-combat ALLY buttons | `ally_toggle: false` | Hides the buttons but does **not** stop automatic deployment; also set `spawn_ally: false` for no allies. |
| Automatic flagship-leaves retreat/handover | `auto_retreat: false` | `flagship_leaves` then has no effect. |
| F8 ally retreat hotkey | `retreat_key: 0` | Automatic and fuel-triggered retreat remain independently configurable. |
| Manual ship switching | `ship_switching: false` | Disables 1/2/TAB handoffs; automatic flagship handover remains unchanged. |
| Low-fuel ally retreat | `ally_fuel_retreat_percent: 0.0` | F8 and flagship-leaves behaviour remain. |
| Ally radio sounds | `ally_sounds: false` | Ordinary log messages remain. |
| Ally AP/proximity ammo doctrine | `ally_ai_ammo: false` | Mod-deployed allies use plain shells. |
| Prize-ship sparing | `spare_prizes: false` | Restores unmodified ally target selection. |
| Ally Zenith autofire | `ally_zenith_autofire: true` | Vanilla: AI-run allies launch their own ship missiles. `false` makes them hold their racks; a ship you fly can still launch manually. |
| Ally ship bombing runs | `ally_bombers: false` | Default off. Restores vanilla ship bomb use; carrier aircraft are independent. |
| Allied damage-number colour | `ally_damage_color: "#A8FC00"` | Restores the stock lime **appearance**. There is no separate off switch for the recolour hooks; use the master switch for original code paths. |
| Battle debug overlay | `debug_overlay: false` | No rings or AI-state labels are drawn. Independent of `spawn_ally` and `ally_reticles`. |
| Total enemy-ship limit | `max_enemy_ships: 5` | Actual vanilla value; the patch is skipped. |
| Ships per enemy wave / wave weight budget | `enemy_group_size: 3` | Actual vanilla value; also leaves the vanilla weight budget unchanged. |
| Initial 30-second enemy-wave delay | `enemy_all_at_once: false` | Vanilla wave timing remains. |
| Carrier loadout buttons, inventory charging and chosen-armament behaviour | `plane_loadouts: false` | Complete revert for the loadout bundle, including guns-only rewriting, rocket-draw suppression, Shipworks stock handling and mod-managed tactical K-13 behaviour. |
| Mod-managed bomb/rocket tactical attacks | `plane_loadouts: false` | `plane_attack_delay: 0.0` removes only the added wait; it does not restore vanilla plane behaviour. |
| Global bomber-routine patch | `plane_bombers: false` | For full plane reversion, keep this false as well as `plane_loadouts: false`. |
| Extra 37 mm gunpod and post-payload strafing | `extra_gunpod: false` | Other chosen-loadout attack behaviour remains while `plane_loadouts` is on. |
| K-13 loadout choice | `plane_k13: false` | Removes the offered choice. `plane_loadouts: false` additionally removes the mod-managed tactical interceptor hook. |
| K-13 launch range | `plane_aa_range: 3750.0` | Actual vanilla range. |
| K-13 alignment tolerance | `plane_aa_angle: 0.1745` | Actual vanilla value, in radians. |
| K-13 speed multiplier | `plane_aa_speed: 1.0` | No speed modification. |
| K-13 engagement delay only | `plane_aa_delay: 0.0` | Removes the added wait, not the mod-managed interceptor; use `plane_k13: false` or `plane_loadouts: false` for the feature. |
| Strategic-launch interception ignore list | `intercept_ignore_oids: []` | Empty list skips the patch and is vanilla. |
| `PLAYER` pre-combat label | `marker_text: ""` | Plane-loadout labels are controlled by `plane_loadouts`, not this key. |
| Marker seam / delay | `marker_delay_ms: 0` removes the extra delay | `marker_seam` has no off value. Disable the owning label/UI feature instead. |
| Spawn/ADD log-line budget | `logging.max_lines: 0` | Minimizes only those lines; it is not a general logging switch. |
| Pre-combat GUI-list log | `logging.log_precombat: false` | Debug mode can force it on. |
| Per-second ally/enemy probe logs | `logging.probe_lines: false` | Debug mode can force it on. |
| Creature registry log | `logging.log_creatures: false` | Other logs remain. |
| GUI click/ammo-selector inspection | `logging.gui_inspect: false` | Other logs remain. |
| HUD colour-call logging | `logging.log_colors: false` | Damage recolouring itself is independent. |
| Debug megalog/watchdog/crash dump | `logging.debug_log: false` | The `megalog` cargo feature forces debug mode on. |
| Dead-node assertion diagnostic | `logging.dead_node: false` | No dead-node hooks installed. |

## Stock-game crash fixes moved to QoL

`fix_plane_ai_crash` and `fix_merge_crash` now belong in `qol.json`, both defaulting
to true. Ally no longer installs these patches; old keys in `ally.json` are ignored.
The bundled QoL build provides them even without ally. Stock upstream QoL does not
include these additions. Update both DLLs together and keep only one of each.
The fleet fixes are null-fleet/vector guards, not a verified fix for all merge crashes.
The battle-restart diagnostic-probe fix remains in ally.

## Logging (`logging` object)

All logging flags and the line budget are grouped in `"logging"`. Old top-level
logging keys remain readable; explicit nested values take precedence per key.
Missing nested keys keep their defaults. The removed `telemetry` key does nothing.
Normal modloader log messages and the basic crash reporter remain available.

```json
"logging": {
  "debug_log": true,
  "max_lines": 600,
  "log_precombat": false,
  "probe_lines": false,
  "log_colors": false,
  "gui_inspect": false,
  "log_creatures": false,
  "dead_node": false
}
```

| Key inside `logging` | Default | Purpose |
|---|---|---|
| `debug_log` | `true` | Megalog `hf_ally_megalog_<time>.log`, watchdog and crash minidump `hf_ally_crash_<time>.dmp`. Dumps may contain paths/campaign data; delete after sending. The `megalog` cargo feature forces this on. |
| `max_lines` | `600` | Spawn/ADD line budget; debug mode lifts it. |
| `log_precombat` | `false` | Pre-combat GUI lists; debug mode can force this on. |
| `probe_lines` | `false` | Per-second ally/enemy activity probes; debug mode can force this on. |
| `log_colors` | `false` | HUD colour-call diagnostic. |
| `gui_inspect` | `false` | GUI lists on clicks and ammo-selector rebuilds. |
| `log_creatures` | `false` | Every creature registry pass (very noisy). |
| `dead_node` | `false` | Read-only, byte-validated hooks reporting the native dead-node/master assertion to `hf_ally_deadnode.log` and the normal log. No CSV capture; does not suppress the assertion. Previously bundled with telemetry builds. |

## Gameplay settings

| Key | Default | If key omitted | What it does |
|---|---|---|---|
| `enabled` | `true` | Uses player default `true`. | Master switch. |
| `spawn_ally` | `true` | Uses player default `true`. | Bring the next ship of the fleet into the battle as an AI wingman. |
| `ally_delay_seconds` | `1.5` | Uses player default `1.5`. | Game seconds the player's ship must have been in the battle before the ally arrives. |
| `auto_retreat` | `true` | Uses player default `true`. | When the flagship leaves alive, perform the action selected by `flagship_leaves`; when false, the game promotes the ally and the retreat point is lost. |
| `flagship_leaves` | `"handover"` | Uses player default `"handover"`. | What `auto_retreat` does when the flagship leaves alive: "retreat" = the ally is extracted with it and the battle ends as a retreat; "handover" = the ally is promoted to your ship (the game's own behaviour) and the retreat point the flagship used up is put back, so the new flagship can still retreat. |
| `retreat_key` | `119` | Uses player default `119`. | Virtual-key code of the "ally, retreat" key (0x77 = F8); 0 disables the key. |
| `ally_toggle` | `true` | Uses player default `true`. | Add the "ALLY" on/off button to the pre-combat screen (needs `spawn_ally`; click it to bring or skip the ally). Clicking it plays the stock `button_01` UI click. |
| `max_allies` | `2` | Uses player default `2`. | How many AI allies may fight at the same time. Ships switched on beyond that wait in the reserve and step in, in ship-number order, when an ally is destroyed or has left. |
| `spare_prizes` | `true` | Uses player default `true`. | Allies never keep a prize ship (the ships the player may capture and sell after the battle: the game flags them at creature +0xc19) as a target: they pick another enemy, or hold fire when only prizes are left. |
| `ally_zenith_autofire` | `true` | Uses player default `true`. | Let AI-run allies launch their own ship missiles (the native Zenith launch path, `0x14002ff60`). `false` = an ally controlled by the native AI holds its missile racks; the ship you fly can still launch manually. Ship bombers always suppress launches while a bombing course owns the ship. Needs `spawn_ally`; enemies, neutrals and player-flown ships are unaffected. |
| `ally_bombers` | `false` | Off. | Experimental FAB1000 **ship** bombing using player-style matched swivel/thrust commands: continuous governed descent (up to 220 u/s, 0.85x the speed the remaining height can recover from), lateral alignment before diving in, release below 1000 units above the target (low spawns already qualify) with real downward speed, and the entire payload rippled on a native predicted hit within the target's bounding radius at 0.05s intervals. Release ceiling is 1000 units above the target (low spawns already qualify) and needs real downward speed; no timer gate and no multi-target switching. Boost pulses 0.5s at `2.0` / 1.0s at normal `1.0`; boost suppressed at max engine temperature350 or when an engine lock is reported. After salvo/unsafe-clearance abort, continue horizontally along the run while pulling up for at least1.5s, until descent is arrested with safe clearance. Recovery survives target destruction. Independent of `hf_ai.json`; flares every0.5s while armed, missile evasion overridden and ship-missile launches blocked during bomber control. Never releases at airborne or prize/trader targets. Requires `spawn_ally`; player-flown ships, enemies and carrier aircraft unaffected. Heat behavior, accuracy and collision safety require in-game validation. |
| `intercept_ignore_oids` | `["MDL_MISSILE_03"]` | Uses player default `["MDL_MISSILE_03"]`. | SIDE PATCH (not in master). `m_oid`s the AI never treats as a strategic launch to intercept in the missile-vs-ship arcade scene, e.g. `["MDL_MISSILE_03"]` (the R-9 Sprint) for a Sprint modded to be strat-launchable (its category has bit 0x8000000, so the CIWS and anti-air launchers of both sides would shoot at it). Copy the names from `m_oid=` in OL.seria. Empty = vanilla. |
| `plane_bombers` | `false` | Uses player default `false`. | Experimental: lets carrier planes with bombs/rockets attack in normal campaign battles (the game only does that in some scenes). Off by default. |
| `extra_gunpod` | `true` | Uses player default `true`. | Normal tactical battle (`Combat.seria`) only, and **only for the vanilla T-7 / LA-29**: give a chosen FAB / NURS plane the vanilla 37 mm gun (`ITEM_GUN37`) and, once both its bombs and rockets are empty, switch it to the interceptor mode so it strafes with that gun until it is dry. Custom crafts (e.g. `CRAFT_SB15`) never get a gunpod from this flag and keep their vanilla leave-after-payload behaviour. If the loadout already has a gun (a class-4 item or a QoL `gun_ammo`, including a custom one), that gun is kept and nothing is added. Custom bombs / rockets are classified by their live ammo-table class, not by name. Strategic map bombers (`Bombers.seria`) keep their vanilla loadout and leave after the bombing run. Off = old behaviour: the plane leaves when its payload is empty. |
| `plane_k13` | `true` | Uses player default `true`. | Offer the K-13 air-to-air loadout for carrier planes. Enabled in the player preset. |
| `plane_attack_delay` | `1.0` | Uses player default `1.0`. | Seconds after launch before a bomb/rocket plane starts its attack runs; until then it flies the game's own script (without a delay LA-29 bombers dive into the carrier deck). |
| `plane_aa_range` | `6000.0` | Uses player default `6000.0`. | Farthest distance (game units) at which a K-13 plane may fire at a target (vanilla window ends at 3750). |
| `plane_aa_delay` | `1.0` | Uses player default `1.0`. | Seconds after launch before a K-13 plane may engage. |
| `plane_aa_angle` | `0.35` | Uses player default `0.35`. | How exactly a K-13 plane must point at the target before firing, in radians (vanilla 0.1745 = 10 degrees; 0.35 = 20 degrees). |
| `plane_aa_speed` | `1.5` | Uses player default `1.5`. | Speed factor for K-13 planes (1.0 = vanilla); the pack uses 1.5 so they can catch missiles and planes. |
| `ally_ai_ammo` | `true` | Uses player default `true`. | Allies roll the AI's AP / proximity shell doctrine once, like enemy ships do at spawn. Default on. |
| `max_enemy_ships` | `8` | Uses player default `8`. | Most enemy ships in one battle (vanilla 5, clamped to 1..=127). |
| `enemy_group_size` | `3` | Uses player default `3`. | Enemy ships placed per wave (vanilla 3, 1..=40). Waves refill every tick after the first 30 s. |
| `enemy_all_at_once` | `false` | Uses player default `false`. | Skip the 30 s wait before the enemy waves start (with `enemy_group_size` > 3: all enemies at once). |
| `plane_loadouts` | `true` | Uses player default `true`. | Pre-combat buttons that pick the armament of a carrier's planes (charged to the inventory), one button per plane type the ship carries, in a column right of the ALLY button. The plane types, their loadouts, item counts and the bomb / rocket / K-13 kind of each item are read from the game's own tables when first needed, so **custom aircraft and re-priced loadouts registered through the QoL `planes` option get buttons too** (up to six plane types per card; a plane with no plain-gun loadout gets the game's 37 mm gun for "guns only"). A chosen loadout carries exactly its listed parts: the vanilla class-3 stock auto-draw is disabled for it, so a FAB plane no longer also draws NAR-122 from another loadout of the same craft. Clicking a loadout button plays the stock `button_01` UI click. Default on. |
| `ally_damage_color` | `"#00E4FF"` | Uses player default `"#00E4FF"`. | Colour of the damage numbers popping up over ALLIED ships (instead of the enemy's yellow-green), as HEX: `"#00E4FF"`, `"00E4FF"` and `"0x00E4FF"` all work. An 8-digit value is read as `AARRGGBB` and its alpha is ignored. A plain number from an old config (e.g. `58623`) is still accepted and means `0xRRGGBB`. |
| `ally_panels` | `true` | Uses player default `true`. | Draw a half-size copy of the ship panel for each ally currently in the battle (at most two), stacked in the empty column right of the player's own panel: name, speed, timer, counters and the live part schematic. They appear only while an ally is present and disappear when the last one leaves. Needs `spawn_ally`; the game's own target panel is unchanged. |
| `ship_switching` | `true` | Uses player default `true`. | **1** takes the displayed top-panel ally, **2** the bottom-panel ally; **TAB** cycles all deployed friendly ships. Your old ship becomes an AI ally. Needs `spawn_ally`; 1/2 also need visible `ally_panels`. See [ship switching](ship-switching.md) for safeguards and keybinding caveats. |
| `ally_reticles` | `true` | Uses player default `true`. | Draw the game's native `sign_aim` target box around the enemy each ally is currently targeting, in the ally colour (`ally_damage_color`). The player's own lime box is unchanged and drawn first; every living ally with a valid target gets its own box. Needs `spawn_ally`. |
| `ally_reticle_scale` | `0.8` | Uses player default `0.8`. | Ally target box size relative to the native 62 × 62 frame: `0.8` is about 50 × 50 pixels, `1.0` is native size. Must be positive and finite; invalid values fall back to `0.8`. Does not affect the player's box. |
| `debug_overlay` | `false` | Off. | Battle debug overlay: three labelled world-space rings (250u / 500u / 1000u) around every live ship, a ship-following label with the native driver state (`+0xbdc`), and the AI steering waypoint with its distance (`WP ...u`, or `DEST ...u` when off-screen). When hf_ai v0.10.9+ is loaded, a second `AI ...` line shows its tactics decision or active flare/evasion mode. Independent of `spawn_ally`; see [debug overlay](debug-overlay.md). |
| `ally_sounds` | `true` | Uses player default `true`. | Radio sounds for ally events: the retreat key, an ally joining the fight, an ally destroyed. |
| `ally_fuel_retreat_percent` | `30.0` | Uses player default `30.0`. | An ally leaves the battle (same order as the retreat key) when its COMBAT fuel (the ship's own tank, the one the battle burns; not the map fuel of the fleet) falls to this percent. 0 = off. Allies only. |
| `marker_text` | `"PLAYER"` | Uses player default `"PLAYER"`. | Text drawn next to ship number 1 on the pre-combat screen (the flagship, always your own ship); "" = none. |
| `marker_seam` | `23` | Uses player default `23`. | Render seam that draws the PLAYER and plane-loadout labels: 22 or 23 (22 may sit under the sliding doors). Legacy 24 is remapped to 23 because the game skips B24 when GLITCH is OFF. |
| `marker_delay_ms` | `0` | Uses player default `0`. | The label stays hidden this many milliseconds after the screen is entered (the doors slide in). |

`intercept_ignore_oids` is a list of part names (`m_oid` from `OL.seria`, case-sensitive) that the AI of both sides never treats as a strategic launch to shoot at, player default `["MDL_MISSILE_03"]` (the stock R-9 Sprint), in both code and the release preset.

Enemy waves: the game spawns enemy ships in waves of `enemy_group_size` (vanilla 3); the first wave comes at the start, the next ones every 30 seconds (or at once with `enemy_all_at_once`). `max_enemy_ships` is the size of the whole list.
