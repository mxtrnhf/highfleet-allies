# hf_ally settings (`Modloader/config/ally.json`)

Every key below is already in the `ally.json` of the release pack, so you do not have to search for flags. A missing key takes the default shown. Edit the file while the game is closed. `"enabled": false` turns the whole mod off.

The checked-in [player preset](../ally/config/ally.json), compiled defaults, and
release config now use the same settings (hf_ally 0.14.2). `debug_log` is `true`
in this preset, so detailed logs/watchdog/crash dumps are active; diagnostic mode
can force probe/pre-combat logging even when those individual keys are false.
Telemetry is false and release DLLs are built without the telemetry feature.
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
| Player/flagship ALLY button | `player_ship_ally: false` | The slot-1 card keeps only the `PLAYER` label; slots 2.. still get ALLY buttons. |
| Automatic flagship-leaves retreat/handover | `auto_retreat: false` | `flagship_leaves` then has no effect. |
| F8 ally retreat hotkey | `retreat_key: 0` | Automatic and fuel-triggered retreat remain independently configurable. |
| Low-fuel ally retreat | `ally_fuel_retreat_percent: 0.0` | F8 and flagship-leaves behaviour remain. |
| Ally radio sounds | `ally_sounds: false` | Ordinary log messages remain. |
| Ally AP/proximity ammo doctrine | `ally_ai_ammo: false` | Mod-deployed allies use plain shells. |
| Prize-ship sparing | `spare_prizes: false` | Restores unmodified ally target selection. |
| Allied damage-number colour | `ally_damage_color: "#A8FC00"` | Restores the stock lime **appearance**. There is no separate off switch for the recolour hooks; use the master switch for original code paths. |
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
| Aircraft target-choice crash fix | `fix_plane_ai_crash: false` | Literal vanilla code, but **not recommended**; true retains a stock-game crash fix. |
| Fleet-merge/empty-vector crash guards | `fix_merge_crash: false` | Literal vanilla code, but **not recommended**; true retains the guards. |
| `PLAYER` pre-combat label | `marker_text: ""` | Plane-loadout labels are controlled by `plane_loadouts`, not this key. |
| Marker seam / delay | `marker_delay_ms: 0` removes the extra delay | `marker_seam` has no off value. Disable the owning label/UI feature instead. |
| Spawn/ADD log-line budget | `max_lines: 0` | Minimizes only those lines; it is not a general logging switch. |
| Pre-combat GUI-list log | `log_precombat: false` | Other pre-combat/toggle messages can remain. |
| Per-second ally/enemy probe logs | `probe_lines: false` | Other logs remain. |
| Creature registry log | `log_creatures: false` | Other logs remain. |
| GUI click/ammo-selector inspection | `gui_inspect: false` | Other logs remain. |
| HUD colour-call logging | `log_colors: false` | Damage recolouring itself is independent. |
| Debug megalog/watchdog/crash dump | `debug_log: false` | Special debug builds can force additional diagnostics. |
| Battle telemetry CSVs and F9 markers | `telemetry: false` | Only present in telemetry-enabled builds. |

| Key | Default | If key omitted | What it does |
|---|---|---|---|
| `enabled` | `true` | Uses player default `true`. | Master switch. |
| `max_lines` | `600` | Uses player default `600`. | At most this many `ALLY` lines are written per level / per session for spawns. |
| `spawn_ally` | `true` | Uses player default `true`. | Bring the next ship of the fleet into the battle as an AI wingman. |
| `ally_delay_seconds` | `1.5` | Uses player default `1.5`. | Game seconds the player's ship must have been in the battle before the ally arrives. |
| `auto_retreat` | `true` | Uses player default `true`. | When the flagship leaves alive, perform the action selected by `flagship_leaves`; when false, the game promotes the ally and the retreat point is lost. |
| `flagship_leaves` | `"handover"` | Uses player default `"handover"`. | What `auto_retreat` does when the flagship leaves alive: "retreat" = the ally is extracted with it and the battle ends as a retreat; "handover" = the ally is promoted to your ship (the game's own behaviour) and the retreat point the flagship used up is put back, so the new flagship can still retreat. |
| `retreat_key` | `119` | Uses player default `119`. | Virtual-key code of the "ally, retreat" key (0x77 = F8); 0 disables the key. |
| `ally_toggle` | `true` | Uses player default `true`. | Add the "ALLY" on/off button to the pre-combat screen (needs `spawn_ally`; click it to bring or skip the ally). |
| `player_ship_ally` | `false` | Uses player default `false`. | Experimental: also show the ALLY button on the player/flagship card (slot 1, the leftmost card). When it is switched ON, that ship is handed to the stock AI at battle start: its AI flag is set, it joins the AI update list, and its player-control link is kept so the camera/HUD/retreat code still treat it as your ship. You do not fly it. Requires `spawn_ally` and `ally_toggle`. Other selected ships still arrive as normal allies. |
| `log_precombat` | `false` | Uses player default `false`. | Log the GUI elements of the pre-combat screen each time it is built (step 1 of the "ally yes/no" button). |
| `probe_lines` | `false` | Uses player default `false`. | Per-second ally/enemy probe and track lines (very chatty; for AI analysis, off for ordinary play). |
| `max_allies` | `2` | Uses player default `2`. | How many AI allies may fight at the same time. Ships switched on beyond that wait in the reserve and step in, in ship-number order, when an ally is destroyed or has left. |
| `spare_prizes` | `true` | Uses player default `true`. | Allies never keep a prize ship (the ships the player may capture and sell after the battle: the game flags them at creature +0xc19) as a target: they pick another enemy, or hold fire when only prizes are left. |
| `intercept_ignore_oids` | `["MDL_MISSILE_03"]` | Uses player default `["MDL_MISSILE_03"]`. | SIDE PATCH (not in master). `m_oid`s the AI never treats as a strategic launch to intercept in the missile-vs-ship arcade scene, e.g. `["MDL_MISSILE_03"]` (the R-9 Sprint) for a Sprint modded to be strat-launchable (its category has bit 0x8000000, so the CIWS and anti-air launchers of both sides would shoot at it). Copy the names from `m_oid=` in OL.seria. Empty = vanilla. |
| `debug_log` | `true` | Uses player default `true`. | Debug mode for bug reports: writes `Modloader/logs/hf_ally_megalog_<time>.log` (large, 2-5 MB/hour), a watchdog and a crash minidump `hf_ally_crash_<time>.dmp` (may hold paths and campaign data: delete it after sending). Enabled in the player preset. |
| `fix_plane_ai_crash` | `true` | Uses player default `true`. | Stock-game aircraft target-choice crash fix. Default on. |
| `plane_bombers` | `false` | Uses player default `false`. | Experimental: lets carrier planes with bombs/rockets attack in normal campaign battles (the game only does that in some scenes). Off by default. |
| `extra_gunpod` | `true` | Uses player default `true`. | Normal tactical battle (`Combat.seria`) only, and **only for the vanilla T-7 / LA-29**: give a chosen FAB / NURS plane the vanilla 37 mm gun (`ITEM_GUN37`) and, once both its bombs and rockets are empty, switch it to the interceptor mode so it strafes with that gun until it is dry. Custom crafts (e.g. `CRAFT_SB15`) never get a gunpod from this flag and keep their vanilla leave-after-payload behaviour. If the loadout already has a gun (a class-4 item or a QoL `gun_ammo`, including a custom one), that gun is kept and nothing is added. Custom bombs / rockets are classified by their live ammo-table class, not by name. Strategic map bombers (`Bombers.seria`) keep their vanilla loadout and leave after the bombing run. Off = old behaviour: the plane leaves when its payload is empty. |
| `plane_k13` | `true` | Uses player default `true`. | Offer the K-13 air-to-air loadout for carrier planes. Enabled in the player preset. |
| `plane_attack_delay` | `1.0` | Uses player default `1.0`. | Seconds after launch before a bomb/rocket plane starts its attack runs; until then it flies the game's own script (without a delay LA-29 bombers dive into the carrier deck). |
| `plane_aa_range` | `6000.0` | Uses player default `6000.0`. | Farthest distance (game units) at which a K-13 plane may fire at a target (vanilla window ends at 3750). |
| `plane_aa_delay` | `1.0` | Uses player default `1.0`. | Seconds after launch before a K-13 plane may engage. |
| `plane_aa_angle` | `0.35` | Uses player default `0.35`. | How exactly a K-13 plane must point at the target before firing, in radians (vanilla 0.1745 = 10 degrees; 0.35 = 20 degrees). |
| `plane_aa_speed` | `1.5` | Uses player default `1.5`. | Speed factor for K-13 planes (1.0 = vanilla); the pack uses 1.5 so they can catch missiles and planes. |
| `fix_merge_crash` | `true` | Uses player default `true`. | Guards against a vanilla crash when fleets merge or land with an empty fleet list (three places). Leave on. |
| `ally_ai_ammo` | `true` | Uses player default `true`. | Allies roll the AI's AP / proximity shell doctrine once, like enemy ships do at spawn. Default on. |
| `max_enemy_ships` | `8` | Uses player default `8`. | Most enemy ships in one battle (vanilla 5, clamped to 1..=127). |
| `enemy_group_size` | `3` | Uses player default `3`. | Enemy ships placed per wave (vanilla 3, 1..=40). Waves refill every tick after the first 30 s. |
| `enemy_all_at_once` | `false` | Uses player default `false`. | Skip the 30 s wait before the enemy waves start (with `enemy_group_size` > 3: all enemies at once). |
| `plane_loadouts` | `true` | Uses player default `true`. | Pre-combat buttons that pick the armament of a carrier's planes (charged to the inventory), one button per plane type the ship carries, in a column right of the ALLY button. The plane types, their loadouts, item counts and the bomb / rocket / K-13 kind of each item are read from the game's own tables when first needed, so **custom aircraft and re-priced loadouts registered through the QoL `planes` option get buttons too** (up to six plane types per card; a plane with no plain-gun loadout gets the game's 37 mm gun for "guns only"). A chosen loadout carries exactly its listed parts: the vanilla class-3 stock auto-draw is disabled for it, so a FAB plane no longer also draws NAR-122 from another loadout of the same craft. Default on. |
| `log_colors` | `false` | Uses player default `false`. | Debug: logs colour calls of the HUD (very chatty). Off. |
| `ally_damage_color` | `"#00E4FF"` | Uses player default `"#00E4FF"`. | Colour of the damage numbers popping up over ALLIED ships (instead of the enemy's yellow-green), as HEX: `"#00E4FF"`, `"00E4FF"` and `"0x00E4FF"` all work. An 8-digit value is read as `AARRGGBB` and its alpha is ignored. A plain number from an old config (e.g. `58623`) is still accepted and means `0xRRGGBB`. |
| `gui_inspect` | `false` | Uses player default `false`. | Log the whole GUI element list (names, positions, sizes) on every click and when the native ammo selector is rebuilt: to learn how the game lays out its own selectors (ammo icons, role icons). |
| `log_creatures` | `false` | Uses player default `false`. | Log every creature registry pass (very noisy: every creature, every frame). |
| `telemetry` | `false` | Uses player default `false`. | Battle telemetry for AI research (only in builds made with the cargo feature `telemetry`; ignored otherwise). |
| `ally_sounds` | `true` | Uses player default `true`. | Radio sounds for ally events: the retreat key, an ally joining the fight, an ally destroyed. |
| `ally_fuel_retreat_percent` | `30.0` | Uses player default `30.0`. | An ally leaves the battle (same order as the retreat key) when its COMBAT fuel (the ship's own tank, the one the battle burns; not the map fuel of the fleet) falls to this percent. 0 = off. Allies only. |
| `marker_text` | `"PLAYER"` | Uses player default `"PLAYER"`. | Text drawn next to ship number 1 on the pre-combat screen (the flagship, always your own ship); "" = none. |
| `marker_seam` | `23` | Uses player default `23`. | Render seam that draws the PLAYER and plane-loadout labels: 22 or 23 (22 may sit under the sliding doors). Legacy 24 is remapped to 23 because the game skips B24 when GLITCH is OFF. |
| `marker_delay_ms` | `0` | Uses player default `0`. | The label stays hidden this many milliseconds after the screen is entered (the doors slide in). |

`intercept_ignore_oids` is a list of part names (`m_oid` from `OL.seria`, case-sensitive) that the AI of both sides never treats as a strategic launch to shoot at, player default `["MDL_MISSILE_03"]` (the stock R-9 Sprint), in both code and the release preset.

`player_ship_ally` is experimental. The ship stays in the game's player slot so the camera/HUD/retreat code still follow it. The mod calls the stock per-ship AI driver for it directly each battle tick instead of adding a Link to the game's AI vector (that Link outlived the ship at scene teardown and crashed the game on exit). Just before the vanilla player update runs, the mod replaces the mouse aim angle with the bearing to the target the AI chose; the update then sends the guns and target links to that target instead of the crosshair. Skipping the update entirely made the AI ship passive, so only the aim value is replaced. The per-battle state is re-armed on the player's spawn, so Shipworks test fights after the first one work too. Mouse input can still move the crosshair/HUD aim ray, but it should not decide where the turrets point.

Enemy waves: the game spawns enemy ships in waves of `enemy_group_size` (vanilla 3); the first wave comes at the start, the next ones every 30 seconds (or at once with `enemy_all_at_once`). `max_enemy_ships` is the size of the whole list.
