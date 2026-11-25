# Shipworks test encounters

**v1.3.47 — Airstrike:** In standalone Shipworks, expand **>** and choose
**A.C.E. → AIRSTRIKE**. **ADD PLANE** adds a group. Click its plane name to
cycle craft types; click a loadout row to equip it. The list shows up to three
options at a time; drag the scrollbar or use the mouse wheel or arrows. The
label includes each weapon's per-plane count, so `2x BOMBS 250` and
`4x BOMBS 250` show up separately. Each page holds four plane groups. Set the number
with the **1–15** slider and multiply it with the **1–8x** slider, up to 120
planes per group. **X** removes one group and **RESET** clears all air groups,
not the ship roster. Aircraft and loadouts come from the live game tables;
modded craft also need an aircraft Body in the game's part library. T-7 and
LA-29 use their stock silhouettes from Midground4; registered modded planes use
the small silhouette from Midground1. **GROUND / AIRBORNE** applies to the
defending ships, not the attacking planes. Airstrike uses the stock Bombers
arena and its limit of five defending ships.

| Added ships | LAUNCH |
|---|---|
| None | Enemy aircraft attack the editor design. |
| Allies only | Enemy aircraft attack the editor design and its allied fleet. |
| Enemies only | Friendly aircraft attack the enemy fleet; watch from spectator view. |
| Allies and enemies | No launch; the panel displays **ONLY ONE TEAM IS SUPPORTED** in red. |

**v1.3.46 — Airstrike damage:** After the raid returns to Shipworks, use
**DMG VIEW** and **PREV / NEXT** to inspect the last native module panel of each
deployed defender. Enable **PRESERVE DAMAGE** before launching to carry survivors'
damage into the next raid and omit destroyed ships. Disable it to launch with
fresh designs; adding or clearing roster ships invalidates saved damage too.
Airstrike and Missile Defense keep separate damage reports and save only
temporary copies, never the original designs.

**v1.3.43 — Silent Strike:** In standalone Shipworks, expand the tools with
**>**, then choose **A.C.E. → SILENT STRIKE**. This opens the usual test
pre-combat screen without a separate setup panel. **ADD** and **ADD ENEMY**
work as in normal **FIGHT**, with the current design on the player's side.
When there is no custom enemy roster, the game uses the normal TEST enemy
settings if available, or its stock empty-roster fallback (an opponent of the
current design). Start the fight from pre-combat: enemies deploy on the ground
as they enter, including reinforcements. Friendly deployment is unchanged.
There is no Preserve Damage or DMG VIEW for this mode.

**v1.3.19 — Missile Defense damage:** Enable **Preserve Damage** before
launching to carry surviving ships and their damage into the next launch with
the same roster. Destroyed ships stay out of subsequent runs. Turn it off before
launching to start fresh; adding a roster entry or clearing the roster invalidates
the replay. The editor design and original roster files are not overwritten.

**v1.3.20 — Static vanilla module panels:** **DMG VIEW**, under **LAUNCH**,
shows the last encounter regardless of the checkbox. **PREV** and **NEXT**
cycle through all deployed ships. The vanilla renderer captures one live
defender per battle frame, including with the HUD hidden as of v1.3.21.
Shipworks shows each ship's last captured panel with stock module art and damage colours. Destroyed
ships keep their last available panel and a **DESTROYED** label; a ship that
never rendered still has a page.

**v1.3.16 — Missile Defense deployment and nukes:** the selector now also has an
**AIRBORNE / GROUND** button above **LAUNCH**.

- **AIRBORNE** keeps the stock four-slot deployment and places additional ships
  in rows beyond those slots.
- **GROUND** uses the native landed mode and, as of v1.3.40, the campaign's
  four defender positions. Campaign Missile Defense deploys at most four
  ships; Shipworks still supports up to 17. Slots 5–17 extend the formation
  beyond the furthest stock slot, with clearance from the missile's entry
  edge. The native ground-height and hull-height clamp still applies.

**v1.3.41 — Nuclear sound and aftermath:** Shipworks lets the native nuclear
sequence finish. It plays `nuke_01`–`nuke_03` at about 3 seconds and stays in
tactical view for about 6 seconds after detonation before capturing damage and
returning to the editor. Conventional encounters keep their normal fade.

**v1.3.42 — Campaign hull damage:** The game's kind-4 detonation never runs
the ordinary tactical area blast. Shipworks uses the campaign's native
falloff, part ordering, and per-column energy consumption instead of sampling
parts and applying a fixed HP budget. The game records a blast distance for
each ship at detonation and uses it in preference to the default map distance.
The closest exposed parts consume the energy first and shield the parts behind
them. Shipworks projects the live ship's geometry without rotating its physics
bodies and applies the resulting HP loss in tactical view for **DMG VIEW**.
It does not process a campaign map, create a save slot, or run rescue or city
events. Only the side of the missile's live linked defender takes hull damage.
See the [blast damage RE notes](https://github.com/mxtrnhf/hf_notes/blob/master/docs/findings/combat/explosion-damage-1151-vs-1163.md).
In the same scene the strategic map-impact call is skipped because it would mutate the
campaign Profile and the staged escadra. The late battle pass that a nuke
enables keeps its effect but selects the stock `ps_mainNoFishEye` technique
instead of the map CRT warp, which fixes the skewed and clipped battle frame.
Flash, cloud and shockwave effects are unchanged.

**v1.3.15 — Missile Defense:** In standalone Shipworks, choose **A.C.E. →
MISSILE DEFENSE**. The panel on the right starts level with **TEST** when
there's room beside the toolbar. Click its first button to cycle through
strategic missiles from the game's live part definitions, then **LAUNCH**. No
**TEST** click is required. Launch starts one missile encounter:

| Added ships | Encounter |
|---|---|
| None | Fly the current editor design against the incoming missile. |
| Allies only | Fly the editor design with the added AI-controlled defenders. |
| Enemies only | Watch the selected missile strike the added enemy ships. The editor design is not deployed. |
| Allies and enemies | Fly the editor design with both rosters present; the missile starts out aimed at your side. |

This uses the game's actual missile-defense scene. Ships deploy at the start
rather than using the normal FIGHT reserve delay and `max_allies` setting.
The editor design and saved roster snapshots remain available on return.

**v1.3.14 — A.C.E. menu preview:** **A.C.E.** sits after **CLEAR** and before
the optional **CRASH** button. It uses TEST's button style and opens a dropdown
with **AIRSTRIKE**, **MISSILE DEFENSE** and **SILENT STRIKE**. In that preview
version each choice only closed the menu.

**v1.3.3 — underlying accumulation bug:** the enemy-append hook copied the
RIP-relative `LEA RCX,[0x143a159c0]` bytes into its trampoline without relocation.
The following native clear therefore received the wrong vector address, leaving
old enemy fleets in the actual list. This also affected fights with untouched
custom rosters. Replay now uses `MOV RCX,imm64` with the original global address.
In `Highfleet_2026-11-14_23-55-13.log`, the retained enemy list grows from one to
six fleets, while custom enemies added remain zero. v1.3.2's null cleanup
prevented the reported crash but did not prevent live fleet accumulation.
HEE's ship-name pool generation is unchanged.

**v1.3.2:** a second **new** FIGHT could crash even without ever using ADD,
ADD ENEMY or CLEAR. v1.3.1 skipped a null enemy-fleet Link in the initial Combat
loader loop, but that Link was subsequently copied into pre-combat and dereferenced
at `0x1402d9806`. The mod now removes dead enemy Links immediately before the
loader's pre-combat copy, for every Shipworks test fight, preserving live fleets
and their order. Compaction uses native Link unregister/copy operations so moved
Links remain registered at their actual addresses; no raw owning-Link moves.
Campaign fights are unchanged.

**v1.2.9:** v1.2.8's prune only scanned the profile escadra vector and never matched in
game, so cleared enemies still leaked into later battles. The prune now scans both the
profile vector (`profile+0x1f8..0x200`, 8-byte cells) and the enemy-side fleet list
(`0x143a159c0`, 0x18-byte link nodes, escadra at node+0x8), and logs where the stale test
escadra was found. `enemy state` log lines at FIGHT entry/build/append, CLEAR and
Shipworks exit dump the stored escadra plus both containers with per-escadra ship counts.

**v1.2.8:** a custom FIGHT's enemy Escadra is registered in the profile's escadra
vector (`profile+0x1f8..0x200`), and the Combat level loader re-links every profile
escadra into the enemy fleet list — so cleared enemies accumulated into later test
battles (2, 10, 18 PRECOMBAT_B cards across consecutive FIGHTs). The mod now prunes
its previous enemy Escadra from that vector at each FIGHT, on CLEAR, and when
leaving Shipworks (bounds-checked scan of pointer values only; the object is never
dereferenced).

**v1.2.7:** the enemy-append trampoline's argument setup mis-encoded the register
move (`49 89 df` = `MOV R15, RBX`, not `MOV RCX, RBX`), so the callback received a
garbage fleet and clobbered R15, crashing the test-combat level load. Fixed to
`48 89 d9` (`MOV RCX, RBX`).

**v1.2.6:** control identifiers must stay within the 15-char inline (SSO)
string storage — `cstr` reads the identifier bytes in place, so the v1.2.5 names
(`SW_ALLY_ENEMY_ADD`, 17 chars) heap-allocated and never matched, leaving the
button recreated every frame and its clicks inert. Enemy controls are now
`SW_ENEMY_ADD` / `SW_ENEMY_1..8` / `SW_ENEMY_STAT`.

**v1.2.5:** **ADD ENEMY** sits immediately right of ADD (CLEAR moves one slot
right) and captures the edited ship into an **ENEMY SHIPS** list rendered below
the ally list in a red-dish text colour (`0xffff5050` ARGB). With a non-empty
enemy list, **FIGHT** overrides the default enemy generation: the enemy Escadra
is created from a single unresolvable design name (no default ships), then each
listed snapshot is appended to it with ship side `-1`. CLEAR and leaving
Shipworks empty both lists and remove all snapshots. Up to eight enemies.

**v1.2.2:** CLEAR sits immediately right of ADD and empties the roster without
leaving Shipworks or changing the edited ship. It also removes the roster's
temporary snapshots and any full-list/capture-error message. You can ADD again
immediately. CLEAR has the same native Shipworks styling and its own click debounce.

**v1.2.1 capture correction:** v1.2.0 skipped the native bridge search inside frame
children and incorrectly took the serialized design root from the Creature's
master rather than the bridge's master. Both paths now match the native save
routine. Failed ADD operations also log the specific capture stage or filesystem
error. The reported v1.2.0 failure was observed in
`Highfleet_2026-11-13_13-11-27.log`; its generic warnings did not identify which
capture guard returned. The maintainer subsequently confirmed v1.2.1 working in game.

In standalone Shipworks, click `>` immediately right of **TEST** to reveal the
mod's tools. Click `<` to hide them again. The tools start collapsed in a new
Shipworks session. Hiding them does not clear the rosters or change which ships
join **FIGHT**.

**ADD** captures the currently edited ship, including unsaved changes. Each press
adds another copy, and the ship names appear below the buttons.

Choose the usual TEST enemy settings and press the existing **FIGHT** button.
The edited ship remains your player ship; the added ships join its friendly
pre-combat roster with ALLY enabled. The normal ally controls, aircraft loadout
buttons and combat ship switching still apply.

- Up to **eight allies**, matching the native nine-player-card screen.
- Added ships follow `ally.json`'s `max_allies` and `ally_delay_seconds`, plus
  the normal three-game-second spacing between deployments. Selected ships above
  the limit wait in reserve. Campaign card choices are unchanged.
- Loading, switching designs, RESET, returning from test combat, and RETRY do not
  clear the list. Returning from the Shipworks landing test also preserves it.
- CLEAR or leaving Shipworks clears both the ally and the enemy list. There is no
  persistent roster, per-ship remove button or additional configuration.
- ADD, ADD ENEMY and CLEAR copy TEST's native backgrounds, dimensions and font. Ally
  list text uses `flash_large_shadow`, the font used for the mod's PLAYER marker; the
  enemy list uses the same font in red-dish `0xffff5050`.
- A full roster or failed capture is reported below the list. A failed capture
  leaves existing entries intact.

Snapshots are temporary `.seria` files under
`Modloader/cache/hf_ally_shipworks/`, named by process ID and a monotonic capture
number. Only files created for the current roster are removed on CLEAR or exit. A game
crash may leave unused files behind; those files are never used to restore a
roster.

## Native evidence (Steam 1.163)

Read-only verification against the live Ghidra program
`Highfleet.exe.unpacked.exe v1.163`, 2026-11-13:

| Address | Contract |
|---|---|
| `0x14024d960` | Despite its existing `ShipDesign_LoadFromFile` name, its body **saves the current editor design**. Finds a `0x40`-flagged part, gets its owner Creature, collects top-level parts plus the master root, and calls SaveVolume. The mod uses that collection/serialization contract without its unrelated unlock/name/screenshot operations. |
| `0x14024d9c4..0x14024da02` | When an editor Body is a frame (`code & 0x1f == 0x1f`), the native bridge search walks its immediate children. Frame-contained parts need not be separate global Body-array entries. |
| `0x14024dad5` / `0x14024ddfe` | RDI is the found bridge, R15 its owner Creature. The save checks the **bridge's** master at `[RDI+0x88]` and appends that Link via `LEA R8,[RDI+0x80]`; it does not append the Creature's master. Reverified against live Ghidra for v1.2.1. |
| `0x14024ecc0` | SaveVolume(path, name, Link-vector): temporarily reparents roots/joints into a native Node, serializes, restores them to the global root, destroys its own temporary Node. Reads but does not own/reallocate the caller's Link vector. |
| `0x140194bd0` / `0x140194d20` | Native Link construction/registration and destruction/unregistration. Snapshot Links use fixed Rust-owned plain storage, never a copied owning `Link` or `CVec`. |
| `0x14013d807..0x14013d8f8` | FIGHT allocates Node `0x128`, loads its test volume (`0x140225930`), calls virtual `+0x80` with zero, assigns recursive random IDs (`0x140280840`), calls virtual `+0x88`, resolves the ship (`0x14021b070`), recalculates stats (`0x14007fcf0`), checks thrust (`0x140073450`), sets morale `+0x1a20 = 10` and side `+0xad4 = 1`. Added snapshots follow the same sequence. |
| `0x14013dc9d..0x14013dcbf` | Creates/initializes the friendly test Escadra and attaches the current player's volume using its native virtual `+0x50`. |
| `0x14013dcc2` | Hook immediately after that attachment, before fleet traversal. RDI is the friendly Escadra. Replay the complete seven-byte `MOV RSI,[RDI+0x100]`; no flags are consumed before the following native CMP. |
| `0x14013df2e` | `Fleet_BuildShopOffer` fills the local enemy ship-name vector from the SMALL/MID/LARGE/CARRIER checkboxes; `0x14013df3c` pushes the player design name when that vector is empty. |
| `0x14013df7e` | `CALL Fleet_CreateEscadraFromShips` builds the enemy Escadra (role in R8D, side/price float in XMM1 = `-1.0` at `0x1403524ec`, position in R9, name and morale on the stack). Replaced by a hook that forwards the original arguments; with a custom enemy roster it swaps the names vector for one unresolvable inline name, so `renderShipPhoto__` (`0x1401c2e80`, returns 0 on a library miss) adds no default ship and the Escadra's native tail still runs. |
| `0x14013df8e` | After the enemy Escadra's init call (`0x1400f9460`), RBX is the enemy Escadra. Hook appends each custom enemy snapshot via the Escadra virtual `+0x50` with ship side `-1.0`, then replays the seven-byte RIP-relative `LEA RCX,[0x143a159c0]` as `MOV RCX,imm64`. The next instruction (`0x14013df95`) calls `Link_ClearVector`; copying the original LEA bytes verbatim changes its target after relocation and fails to clear the real enemy list. |
| `0x1401c7060` | `Fleet_CreateEscadraFromShips` entry (byte-checked): allocates the 0x618 Escadra, copies the name, adds one resolved ship per 0x20 name element, then runs the role/profile/proximity tail. |
| `0x14025aff4` | SetLevelSeriaToLoad(path, flags), after the prologue and before the path copy. The hook preserves the arguments and replays `MOVZX EDI,R8B; MOVZX ESI,DL; MOV RBX,RCX`; RSP is already aligned. This leaves the entry at `0x14025afd0` available to Map Mode Extended. `Combat.seria`, test `City_generated.seria`, and return `Editor.seria` belong to the same session; another level ends it. |
| `0x140292a90` | `Get_Profile_()` (byte-checked). Its escadra vector (`profile+0x1f8` begin, `+0x200` end) is read by the Combat level loader (`0x1402c3e49..0x1402c3f61`), which re-links each profile escadra into `0x143a158e0`/`0x143a159c0`. The mod erases its previous test enemy Escadra pointer from this vector before each FIGHT / on CLEAR / on session end. |
| `0x1402c3a98` | Byte-checked `LEA RDX,[0x143a159c0]`, before `VectorLink_CopyConstruct` at `0x1402c3aa6` and `buildSquadPrecombatScreen` at `0x1402c3ab1`. New cleanup seam removes null enemy Links before the copy. Replays the RIP-relative LEA as `MOV RDX,imm64`; preserves other volatile registers and does not depend on incoming flags. Verified read-only in live Ghidra, 2026-11-14. |
| `0x1402d9802..0x1402d9806` | `Precombat_SplitFleetIntoReserveLists` reads `[R13+8]` then `[RAX+0x100]` without a null check. The 2026-11-14 23:37:56 log crashes here on the second FIGHT; RAX is zero and the copied vector spans two Link slots. The earlier `0x1402c3a00` guard alone cannot protect this consumer. |
| `0x140194b80` / `0x140194d20` | Native Link copy construction registers the destination address with its Node; destruction unregisters that address. Retention unregisters each overwritten destination, copy-constructs live Links in order, destroys the discarded tail, then shortens end. Source Links remain initialized until overwritten or destroyed. Both call entries are byte-checked. |
| `0x1401351b7..0x1401353d3` | LANDING test sets the editor-test flag `0x147fc6fb6` and requests **City_generated.seria**, not a hypothetical Landing.seria. |

Existing complete-list GUI membership/root checks protect retained GUI addresses;
no retained editor Creature/part pointers survive a callback. Native GUI
constructors, initializers, string assignment and resource loading own all
game-side allocations. Covered offsets/sizes derive from the pinned highfleet-rs
layouts; `Creature+0xa90` (design name), `+0x1a20` (initial morale), editor globals
and GUI geometry `+0x218/+0x21c` remain locally documented RE contracts.

### Airstrike (Steam 1.163)

| Address | Contract |
|---|---|
| `0x143a13c50`, `0x143a15a60`, `0x14021b190` | The live craft map holds each craft's vector of loadout pointers. Native `DefinePart` clones the chosen craft from `ALL_PARTS`, and arming looks up its chosen loadout key at `Body+0x3f8`. The UI re-reads the catalog before launch. |
| `0x1400ceade`, `0x14016b9c0`, `0x14016bf00` | Use the native editor Slider recipe (`0xea8` bytes), including its own drag handler and thumb movement. The panel reads the value only from a live GUI member. |
| `0x1402821c0`, `0x140170230`, `0x140303fc2` | Resolve `Stats.m_icon_name` to the registered media animation. In the editor label pass, `Default_ps.hlsl`'s `ps_add` with white vertex RGB makes a silhouette on the normalized target while preserving texture alpha. The submitted quad is local; the shared animation and texture are not modified. |
| `0x1402c5540` → `0x1402ca2b0` | State 1 expects a `MapCraft` with plane Bodies in side A and the defending Escadra in side B. The temporary MapCraft is constructed natively (`0x1400e4530`), never registered in the profile, and used only by this Shipworks scene. The stock group spawner at `0x140267700` sets alignment and arms the planes. |
| `0x1402d2e61`, `0x1402c5357`, `0x1402c5340` | Set the defender Escadra's altitude only during synchronous setup (zero for GROUND). In the test scene run the stock raid tick without campaign tutorial events, capture damage before teardown, skip campaign map teardown, then route the shared exit back to Editor. Clear temporary Links with native unregistration before deleting the private MapCraft. The tactical world retains ownership of spawned ships and planes. |
| `0x147fc73d8`, `0x1402cb386` | Capture only the matching live Creature links in the Bombers defender vector (maximum five), not planes or the missile roster. The wave-spacing hook loads the stock 2,500-unit increment for campaign Bombers and reduces it for large Shipworks raids. |

### Silent Strike (Steam 1.163)

| Address | Contract |
|---|---|
| `0x14013d7b7` / `0x14013dcf1` / `0x14013e05e` | Alias an accepted Silent Strike click to native FIGHT. Use its normal enemy generator if the TEST controls exist, otherwise enter its empty-list check at `0x14013df38`. Commit the mode only at the final `Combat.seria` request, after native design validation and roster linking. |
| `0x1402d2498` → `0x1402d24bf` | In `Battle_DeployFromReserve`, take the native campaign ground branch only for a negative-side template in a Silent Strike test. Native placement searches the terrain and `0x1402d2bc3` broadcasts deployment command `3` when it finds a valid spot. If there is no room, the stock spawner retains its placement fallback. |
| `0x1402d1bc2` → `0x1402d1bf5` | In `BattleSpawner_SpawnShips`, take the corresponding native ground branch for enemy waves; `0x1402d21da` broadcasts the same command. The hooks change neither the wave budget nor the friendly roster. |

### Missile Defense (Steam 1.163)

Read-only Ghidra findings for the new adapter:

| Address | Contract |
|---|---|
| `0x143a33bf0`, `0x140281e00`, `0x140281fa0` | Live `Stats*` vector and native OID lookup. OID at `Stats+0x48`, category bit `0x08000000` is strategic launch, homing mode and yield at `+0x164` / `+0x168`. `DefinePart` (`0x14021b190`) resolves that OID through `ALL_PARTS` (`0x143a139f0`) virtual `+0xc8`; the picker requires a matching Body there. Native localization is at `0x1401ae090`. |
| `0x14013d7b7`, `0x14013dcf1`, `0x14013e05e` | An accepted LAUNCH matches the FIGHT click branch, which saves and clones the editor design. Skip the default enemy generator (it expects the TEST checkboxes). At the final loader call use the staged test fleets and request `Missile.seria` instead of `Combat.seria`. Ordinary FIGHT uses the original compare, generator and level. |
| `0x1402c4dfc` → `0x1402d45c0` | SCR_MISSILE state 1 expects the defenders in A (`0x143a158e0`) and the map Missile in B[0] (`0x143a159c0`). For the test, allocate and construct a temporary Missile with the native `0x1400e48a0` (`0x308` bytes, as in the strategic launch caller `0x1400ecf80`). Do not add it to any profile. It lives through the tactical scene because logic 10 links back to it. |
| `0x1402d4902` / `0x1402d46a1` / `0x1402d4b21` / `0x1402d4c7e` | In the custom scene preserve roster order rather than sorting by anti-air score, force the native airborne/landed choice for AIRBORNE/GROUND, place ships beyond the first four before the native ground-height clamp, and bound the loop to the two Shipworks rosters plus the editor ship. `0x1402d4b21` must keep `MOVSD [RBP+0x67],XMM0`. Native world-list registration reads the ship's AI flag, so set it before spawning each staged template. |
| `0x1402d4ae2` / `0x1402d4dde` | The same direction byte selects opposite edges for the stock defender formation and incoming missile. The first GROUND defender is 2,000 units in from the defender edge, leaving 4,000 units of horizontal approach on the stock 6,000-unit-wide arena. The old custom GROUND row put it 450 units from the missile edge instead. |
| `0x140029c02` → `0x14002bb50` | `Body_Explode` (`0x140029b60`) sends kind 4 through the nuclear handler and exits without the ordinary area pass. Shipworks reads the warhead yield from `Body+0x4b0` → the staged `Missile+0x2f0`, not `Body+0x2dc` (logic 10 overwrites that with `100`). The defender Link at `Body+0x498` must identify a live opposing roster member before the sandbox applies fleet damage. |
| `0x14002bd72` → `0x1400f11b0` | The nuclear handler's strategic impact call adds a Profile `BLAST` mark and can raise escalation. It is skipped while the Shipworks scene is active; the mark would otherwise deal damage later on the campaign map. |
| `0x14002be2c`, `0x1400e6aa0`, `0x1402123e0`, `0x1400e2d00`, `0x1400fd990` | Native detonation writes each creature's distance divided by 1,000 to `+0xd58`. The campaign ship routine prefers it to the map distance, uses the native falloff, and sorts parts by their front edge in the blast's frame. It spends energy in one-unit columns using part HP and armor category, then subtracts HP directly. Shipworks borrows the stock math, sort, and spending routines on bounded scratch records, uses current tactical part geometry, and queues part deaths without calling the map-only scrap/recovery tail. |
| `0x1402d4bdf` → `0x140079b20` | AIRBORNE uses the native retract command, then snaps the landing gear into the folded pose via `ShipLink_SnapAllBodies` (`0x1401bccb0`), like the normal combat spawner (`0x1402d21f0`). GROUND never reaches this call, and campaign Missile Defense only replays the original command. |
| `0x1402c5067`, `0x140040dd5`, `0x140040ed8`, `0x140040ee9` | With the native nuke-effect flag set, SCR_MISSILE enters state 10 and leaves the finish to the logic-`0x1518` effect. The effect plays randomized stock `nuke_01`–`nuke_03` when its age crosses 3 seconds. At 6 seconds it calls missile teardown, then requests `Map.seria`. Shipworks captures before that teardown and substitutes `Editor.seria` only for its own encounter. |
| `0x140309b94` | A nuke makes the late battle pass apply `ShaderMapPostProcessing`'s `ps_main` (`+0x118`) to the whole frame, including its `pullPower` trapezoid and 0.9 scale. The hook reads the technique pointer from the shader in `RCX` and selects `+0x120` (`ps_mainNoFishEye`) only in the Shipworks scene. |
| `0x1402d4e80` → `0x140263700` | The original call configures logic 1 for `_CONV` and 10 otherwise. The custom call uses that split but explicitly picks a defender on the opposite side and binds native Links. Logic 10 (`0x140037fb7`) only homes on that target when the map missile is within 100 map units of its waypoint (`0x1400f8e00`); both map vectors start equal for the test. |
| `0x1402c50f1` → `0x1402d4f20` | Save before teardown clears the live roster and moves the ships to map properties. Native `SaveVolume` (`0x14024ecc0`) writes surviving assemblies, including part health. Match surviving ships by combat roster identity and Node ID, not by name or position. |
| `0x140309a9a`, `0x1402fe09a` | Capture one live defender into a private 350×300 target before the stock panel call. Capture runs before the branch that skips panel rendering when the HUD is hidden. Use the stock mode-2 renderer and substitute the ship lookup only during the mod's draw, without changing the player or target links. After capture the native panel pass rebinds its own targets; the separate stock HUD composite still respects the real HUD flag. |
| `0x140303fc2` → `0x140083b50` | Composite the last captured native panel before the editor label target flush, without accessing combat objects. This does not replace QoL's elevation hint at `0x140303fb4`. |
| `0x1402c5102`, `0x14025aff4` | Native SCR_MISSILE teardown is followed by a Map request; send the test back to `Editor.seria`. The loader seam clears the staged Links and calls the map Missile's deleting destructor (`0x1400e81d0`) only when the current B[0] still identifies the object the mod allocated. Native campaign Missile paths still use their original calls. |

## Validation

Airstrike (v1.3.44, in-game pending): start Steam 1.163 with the versioned
DLL, open **A.C.E. → AIRSTRIKE** without opening **TEST**, and add two groups
with different craft/loadout choices. Check the monochrome icons, slider
endpoints, zero-count group and **RESET**. Repeat with a mod-registered craft
and loadout. Launch with each of the four roster combinations above, try both
deployment settings, check a repeat launch and Esc → Return to Shipworks, then
run ordinary **TEST → FIGHT** to check that its roster and deployment are
unchanged. The `Shipworks Airstrike` and `PLANE arm` log lines identify setup
and the loadouts passed to native arming.

Silent Strike (v1.3.43): launch it before opening **TEST**, then repeat with
configured TEST settings, allies only, custom enemies only, and both rosters.
Verify the usual pre-combat cards, unchanged friendly deployment, and landed
enemies at spawn (including a later wave). Back out from pre-combat and repeat
after a completed fight; follow either return with an ordinary **TEST → FIGHT**
to check that the landed setting does not carry over.

v1.3.16 needs a Steam 1.163 playtest. Open Shipworks and launch directly,
without opening TEST. Check button placement and text fit for the missile type,
AIRBORNE/GROUND and LAUNCH rows, and cycle through conventional and nuclear
options. Try all four roster rows above once with AIRBORNE and once with
GROUND; use at least five total ships in one run to exercise the extra slots.
On GROUND, confirm the first ship has room to react to an incoming conventional
missile from either side, and that the fifth and later defenders still start on
the terrain and clear of the entry edge. Impact a nuclear missile and verify
ships and parts take blast and radiation damage, the battle frame stays
unwarped and unclipped during the flash/cloud pass, and the scene still
returns naturally. Also verify AI defense,
missile homing and interception, spectator view with enemies only, Esc → Return
to Shipworks, and a second launch. Finish with an ordinary TEST/FIGHT to check
that nothing from Missile Defense leaked into its roster. If anything goes
wrong, keep the newest `Highfleet_*.log` (look for `Shipworks Missile Defense`
and `Shipworks missile ... armed`).

For damage, launch with **Preserve Damage** enabled, damage a survivor and
destroy another ship, then return to Shipworks. Verify that **DMG VIEW**
shows both ships with stock module art and frozen colours; the destroyed
ship's page should retain its last available panel and display **DESTROYED**. Launch again with the same
roster and check that only survivors return and retain their damage. Repeat
with the checkbox off and after changing the enemy list, and check that normal
TEST/FIGHT still uses the original designs.

For the landing gear, watch a leg-equipped defender from the first AIRBORNE
frame and check that its gear is folded. Repeat in GROUND to verify the stock
deployed pose and use the GEAR control to check that deployment still works.

To check the nuclear and spectator fixes, add three enemies (no allies) and
launch `MDL_NUKE_01` in AIRBORNE mode. After detonation, verify it returns to
Shipworks without GAME OVER and that DMG VIEW shows captured panels. Repeat
with `MDL_NUKE_01_CONV` to check the normal return still works. The capture log
reports native panel count and how many frames ran with the HUD hidden.

For nuclear blast balance, repeat the KH-15N (`MDL_NUKE_01`) Strike / AIRBORNE
run with six fresh Morals, then try A-100N (`MDL_NUKE_03`) and a conventional
missile against the same fresh roster. Leave Preserve Damage off for the
comparison and check DMG VIEW after each run. For the stock nuclear missiles,
the detonation log should report warhead yield `40000000` and the target side.
The geometry lines report tactical distances; they do not define the ships
eligible for damage. Check the `sandbox nuke ship` lines for per-ship HP loss
and destroyed parts. Closer ships and exposed modules can take more damage;
there is no fixed per-ship damage budget. Verify the delayed explosion sound, roughly six seconds of
tactical aftermath, and that DMG VIEW is available on return. For a Mixed
roster check that only the target side logs nuclear fleet damage; for a P
missile with no emitter, check that no fleet damage is applied. To check saved
HP, enable Preserve Damage before a fresh run and inspect the `.seria` files
afterward.

v1.3.25 adds Preserve Damage diagnostics (no behavior change). Launch twice
with Preserve Damage on and read the newest `Highfleet_*.log`:
`Shipworks preserve replay: N saved`, `staged preserved snapshot <ship>:
A/B parts below max health`, `spawned <ship> as <slot> with A/B parts below
max health`, and `capturing <ship> with A/B parts below max health`. The
combination shows whether the next launch consumed the volumes, whether the
loaded ship still had damage, and whether spawning or capture lost it.
v1.3.26 adds the summed hull HP to those lines plus the distance from the
detonation to each live defender. The logged equivalent tactical radius is for
comparison only; the fleet damage model does not use it.

v1.3.28 adds log-only campaign nuke instrumentation (no behavior change).
A kind-4 detonation logs `campaign nuke detonation` with the projectile power,
linked map missile, warhead yield and position. The strategic handler logs
`campaign nuclear impact` with the target yield/position, scale, angle and
escalation, then `campaign nuclear impact done` and one `campaign explosion
mark` line with `power`, `radius`, `yield`, `nuclear` and `ship damaged`.
Capture the newest `Highfleet_*.log`, plus `client.log` and `error.log` from
the game root, and state what happened on the map (ship loss, city damage,
GAME OVER, escalation). A vanilla run without this build cannot answer the
campaign nuke question because the stock logs do not record it.

v1.3.14: The maintainer confirmed the A.C.E. menu working in game. For retesting
in standalone Shipworks, verify order, font, and all three dropdown rows
with CRASH enabled and disabled. Check that a second A.C.E. click, choosing an
option, or clicking another control dismisses the list. Leave and re-enter
Shipworks with the list open and confirm it starts closed. Check that normal
TEST/FIGHT behavior is unchanged after choosing a placeholder.

v1.3.13: Shipworks deployment-limit changes require in-game validation. Set
`max_allies` below the number of added allies and a nonzero `ally_delay_seconds`;
verify the initial delay, deployment spacing, capacity, and reserve replacement,
then check campaign combat and Shipworks RETRY.

v1.3.9 (2026-11-16): the maintainer reports that Map Mode Extended and allies
work together in game and that no Shipworks problems were detected. Specific
Shipworks transitions and load orders were not enumerated in the report.

v1.3.3 (2026-11-15): locked no-default-features Windows release build, targeted
rustfmt and diff checks passed. Default/megalog Windows release Clippy passed
with the same 22 existing warnings. Audited the other Shipworks replays: fleet
and level instructions have no RIP-relative operands; the pre-combat cleanup
already uses an absolute address, and the enemy-loop MOVs are register-relative.
No new tests or dependency changes. **Confirmed working in game by the maintainer.**
The 2026-11-15 00:09:55 debug session records eight FIGHT starts, including repeated
ordinary fights, two custom-roster fights and CLEAR. The prior enemy list remains
at one entry before replacement instead of accumulating; after CLEAR it is empty.
The original second-sortie access violation does not recur. A separate exception
after TITLE_EXIT is outside this gameplay validation.

v1.3.2 (2026-11-14): locked release Windows cross-build passed; default and
legacy-`megalog` Windows release Clippy passed with the same 22 existing warnings,
none in `shipworks.rs`. Targeted rustfmt and `git diff --check` passed. Full
`cargo fmt --check` still reports pre-existing differences in other modules.
No regression tests were added; **the second-new-sortie fix requires in-game
retesting**. HEE and live configurations were not changed.

Windows cross-build succeeds. The maintainer confirmed v1.2.1 working in game.
**The new CLEAR button needs in-game verification**; compilation alone does not
validate its placement or behavior. **The v1.2.5 ADD ENEMY button and enemy-fleet
override need in-game verification.** No regression tests were added.

Release Windows Clippy with `megalog` completes with the existing 22 warnings
(none in the new feature files). Changed Rust files pass targeted rustfmt checks;
full `cargo fmt --check` still reports pre-existing differences in other files.
The no-default-features local build is
`ally/target/x86_64-pc-windows-msvc/release/hf_ally_v1.3.3.dll`.

Manual pass on Steam 1.163:

1. Open Shipworks. Check ADD/CLEAR alignment and fonts at the supported screen sizes.
2. ADD one saved design, modify it without saving, and ADD again. Check both names
   and that the two snapshots retain their distinct builds.
3. Load a different design, switch designs and RESET; the list must remain.
4. FIGHT with two or more entries. Check the current design remains the player
   ship and every added copy is friendly, AI-flown and switchable.
5. Retry, then return to Shipworks; the list and snapshot builds must remain.
6. Run/return from LANDING; the roster must remain.
7. Fill all eight slots and attempt another ADD; no ninth ally should be added.
8. Leave Shipworks, re-enter, and check the list is empty and the session files
   were removed. Start a campaign fight and verify its previous ally choices,
    `max_allies` and deployment delay are restored.
9. ADD several ships, then CLEAR. Check that names/status disappear, snapshots are
   removed and the edited ship is unchanged. ADD another design immediately and
   FIGHT; only the new entry should join.
10. ADD ENEMY one or more designs (check the red list text), then FIGHT. The test
    combat's enemy side must be exactly the listed ships — no default SMALL/MID/LARGE/
    CARRIER generation — and the player ship plus any ADD allies stay friendly.
11. ADD ENEMY, then CLEAR; check both lists and all snapshots are gone. Leave
     Shipworks with an enemy list pending; re-enter and check the list is empty. Also CLEAR an empty/full/error roster.
12. Without touching ADD, ADD ENEMY or CLEAR, finish a test battle, decline RETRY,
    return to Shipworks and start a **new** TEST/FIGHT. Repeat at least three times;
    each fight must show fresh enemy cards and start normally. Repeat with HEE
    shipworks edits enabled, then with custom ally/enemy rosters and after CLEAR.
    Check `removed ... dead/excluded enemy fleet links` diagnostics when dead Links
    are present. v1.3.3's accumulation fix was confirmed in game on 2026-11-15.
