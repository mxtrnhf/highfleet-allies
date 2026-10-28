# hf_ally settings (`Modloader/config/ally.json`)

Every key below is already in the `ally.json` of the release pack, so you do not have to search for flags. A missing key takes the default shown . Edit the file while the game is closed. `"enabled": false` turns the whole mod off.

Pack values that differ from the code defaults: `max_enemy_ships` 8 (code 5), `plane_k13` true (code false), `ally_fuel_retreat_percent` 30 (code 0), `probe_lines` false, `log_precombat` false, `plane_aa_delay` 1.0 (code 3.0), `plane_attack_delay` 1.0 (code 6.0).

| Key | Default | What it does |
|---|---|---|
| `enabled` | `true` | Master switch. |
| `max_lines` | `600` | At most this many `ALLY` lines are written per level / per session for spawns. |
| `spawn_ally` | `true` | Bring the next ship of the fleet into the battle as an AI wingman. |
| `ally_delay_seconds` | `1.5` | Game seconds the player's ship must have been in the battle before the ally arrives. |
| `auto_retreat` | `true` | When the flagship leaves the arena alive, order the ally out too (otherwise the battle goes on with the ally promoted to your ship and the retreat is lost). |
| `flagship_leaves` | `"handover"` | What `auto_retreat` does when the flagship leaves alive: "retreat" = the ally is extracted with it and the battle ends as a retreat; "handover" = the ally is promoted to your ship (the game's own behaviour) and the retreat point the flagship used up is put back, so the new flagship can still retreat. |
| `retreat_key` | `119` | Virtual-key code of the "ally, retreat" key (0x77 = F8); 0 disables the key. |
| `ally_toggle` | `true` | Add the "ALLY" on/off button to the pre-combat screen (needs `spawn_ally`; click it to bring or skip the ally). |
| `log_precombat` | `true` | Log the GUI elements of the pre-combat screen each time it is built (step 1 of the "ally yes/no" button). |
| `probe_lines` | `true` | Per-second ally/enemy probe and track lines (very chatty; for AI analysis, off for ordinary play). |
| `max_allies` | `2` | How many AI allies may fight at the same time. Ships switched on beyond that wait in the reserve and step in, in ship-number order, when an ally is destroyed or has left. |
| `spare_prizes` | `true` | Allies never keep a prize ship (the ships the player may capture and sell after the battle: the game flags them at creature +0xc19) as a target: they pick another enemy, or hold fire when only prizes are left. |
| `intercept_ignore_oids` | `[]` | SIDE PATCH (not in master). `m_oid`s the AI never treats as a strategic launch to intercept in the missile-vs-ship arcade scene, e.g. `["MDL_MISSILE_03"]` (the R-9 Sprint) for a Sprint modded to be strat-launchable (its category has bit 0x8000000, so the CIWS and anti-air launchers of both sides would shoot at it). Copy the names from `m_oid=` in OL.seria. Empty = vanilla. |
| `debug_log` | `false` | Debug mode for bug reports: writes `Modloader/logs/hf_ally_megalog_<time>.log` (large, 2-5 MB/hour), a watchdog and a crash minidump `hf_ally_crash_<time>.dmp` (may hold paths and campaign data: delete it after sending). Default off. |
| `fix_plane_ai_crash` | `true` | Stock-game crash fixes (aircraft target choice, fleet-merge null fleet). Default on. |
| `plane_bombers` | `false` | Experimental: lets carrier planes with bombs/rockets attack in normal campaign battles (the game only does that in some scenes). Off by default. |
| `extra_gunpod` | `true` | Normal tactical battle (`Combat.seria`) only: give a chosen FAB / NURS plane the vanilla 37 mm gun (`ITEM_GUN37`) and, once both its bombs and rockets are empty, switch it to the interceptor mode so it strafes with that gun until it is dry. Strategic map bombers (`Bombers.seria`) keep their vanilla loadout and leave after the bombing run. Off = old behaviour: the plane leaves when its payload is empty. |
| `plane_k13` | `false` | Offer the K-13 air-to-air loadout for carrier planes. Default off (janky). |
| `plane_attack_delay` | `6.0` | Seconds after launch before a bomb/rocket plane starts its attack runs; until then it flies the game's own script (without a delay LA-29 bombers dive into the carrier deck). |
| `plane_aa_range` | `6000.0` | Farthest distance (game units) at which a K-13 plane may fire at a target (vanilla window ends at 3750). |
| `plane_aa_delay` | `3.0` | Seconds after launch before a K-13 plane may engage. |
| `plane_aa_angle` | `0.35` | How exactly a K-13 plane must point at the target before firing, in radians (vanilla 0.1745 = 10 degrees; 0.35 = 20 degrees). |
| `plane_aa_speed` | `1.5` | Speed factor for K-13 planes (1.0 = vanilla); the pack uses 1.5 so they can catch missiles and planes. |
| `fix_merge_crash` | `true` | Guards against a vanilla crash when fleets merge or land with an empty fleet list (three places). Leave on. |
| `ally_ai_ammo` | `true` | Allies roll the AI's AP / proximity shell doctrine once, like enemy ships do at spawn. Default on. |
| `max_enemy_ships` | `5` | Most enemy ships in one battle (vanilla 5, clamped to 1..=127). |
| `enemy_group_size` | `3` | Enemy ships placed per wave (vanilla 3, 1..=40). Waves refill every tick after the first 30 s. |
| `enemy_all_at_once` | `false` | Skip the 30 s wait before the enemy waves start (with `enemy_group_size` > 3: all enemies at once). |
| `plane_loadouts` | `true` | Pre-combat buttons that pick the armament of a carrier's planes (charged to the inventory), one button per plane type the ship carries, in a column right of the ALLY button. The plane types, their loadouts, item counts and the bomb / rocket / K-13 kind of each item are read from the game's own tables when first needed, so **custom aircraft and re-priced loadouts registered through the QoL `planes` option get buttons too** (up to six plane types per card; a plane with no plain-gun loadout gets the game's 37 mm gun for "guns only"). Default on. |
| `log_colors` | `false` | Debug: logs colour calls of the HUD (very chatty). Off. |
| `ally_damage_blue` | `true` | Damage numbers popping up over ALLIED ships are drawn in the ally colour instead of the enemy colour. |
| `ally_damage_color` | `58623` | The ally damage colour as a plain number 0xRRGGBB (58623 = 0x00E4FF, cyan). |
| `gui_inspect` | `false` | Log the whole GUI element list (names, positions, sizes) on every click and when the native ammo selector is rebuilt: to learn how the game lays out its own selectors (ammo icons, role icons). |
| `log_creatures` | `false` | Log every creature registry pass (very noisy: every creature, every frame). |
| `telemetry` | `false` | Battle telemetry for AI research (only in builds made with the cargo feature `telemetry`; ignored otherwise). |
| `ally_sounds` | `true` | Radio sounds for ally events: the retreat key, an ally joining the fight, an ally destroyed. |
| `ally_fuel_retreat_percent` | `0.0` | An ally leaves the battle (same order as the retreat key) when its COMBAT fuel (the ship's own tank, the one the battle burns; not the map fuel of the fleet) falls to this percent. 0 = off. Allies only. |
| `marker_text` | `"PLAYER"` | Text drawn next to ship number 1 on the pre-combat screen (the flagship, always your own ship); "" = none. |
| `marker_seam` | `24` | Render seam that draws the label: 22, 23 or 24 (24 is the topmost; a lower one may sit under the sliding doors). |
| `marker_delay_ms` | `0` | The label stays hidden this many milliseconds after the screen is entered (the doors slide in). |

`intercept_ignore_oids` is a list of part names (`m_oid` from `OL.seria`, case-sensitive) that the AI of both sides never treats as a strategic launch to shoot at, default empty in code and `["MDL_MISSILE_03"]` (the stock R-9 Sprint) in the pack.

Enemy waves: the game spawns enemy ships in waves of `enemy_group_size` (vanilla 3); the first wave comes at the start, the next ones every 30 seconds (or at once with `enemy_all_at_once`). `max_enemy_ships` is the size of the whole list.
