# Strategic shelling

Install CCU and leave `"enable": true` in `Modloader/config/ccu.json` before
starting the game. CCU can run by itself. On the campaign map,
select a player fleet with at least one working 180 mm gun. Click **SHELLING**
above the map coordinates, then left-click a visible city marker. Click the
estimate to cycle 10, 30, and 60 strategic minutes, then **FIRE** to start the
barrage. **CANCEL** leaves the preview; while a barrage runs its fleet shows
**ABORT** instead.

The first build uses a 180 mm mount as the eligibility rule, without a separate
cruiser class requirement. Standard HE is unlimited: the displayed number is
the estimated rounds fired, not an inventory cost. The volume is the sum of
each mount's output using its live rate, magazine and reload. The estimate
assumes a full starting magazine, reloads one round at a time, and skips
tactical crew/perk modifiers. The range (270 km landed, 450 km at cruise
altitude), travel time and hit model are provisional **game balance** values,
not real-world ballistics. ETA is the first round's flight time at a fixed
strategic speed of 1 km/s (five seconds minimum). It depends only on the
distance when the fire order is issued, not on the firing window, battery
size or altitude. Subsequent fleet movement changes tracer origins, not
the barrage's arrival schedule or hit odds.

Each shell has a small, range-dependent chance of damaging a useful target
inside the city; most rounds land in the built-up area and count for nothing.
The label shows **expected hits**, which grows with rounds, and appends the
chance of at least one hit while that chance is below 99.5%. That chance is
`1 - (1 - p)^rounds`, so longer barrages always land at least as many expected
hits.

**FIRE prepares a single firing plan.** After the servo and crew cues, the
campaign clock starts the firing window. Each round has an effective-hit result
from the range-dependent probability shown by the preview. The map display
samples that plan. Its tracers travel from the fleet's position at launch and
do not follow it after launch. All effective rounds count when the last shell
arrives, even if time compression jumped past their visible impacts.
The firing window lasts the selected 10, 30 or 60 minutes; the last shells
are still in flight when it ends. The map shows **LANDING** until they arrive.

At the end of an uninterrupted barrage with effective hits and a hostile
garrison, the tactical scene copies the defenders' starting module positions
and health and resolves effective rounds against those copies under the BLACK
overlay. After the simulation completes, the overlay fades out during a
2.6-second wind-only opening. The replay shows the pre-barrage ships. The fixed shot gives most of
the frame to the sky and places sampled ground near 93% of the viewport height.
The native impact effect supplies flash, smoke and flying debris. Representative
dirt strikes are spread over clear terrain in the shot and are at least twice
as frequent as hull hits, up to 30 displayed ground strikes. Misses
are cosmetic and do not consume an effective strategic hit. Quick salvos
and occasional single rounds alternate with longer, uneven lulls. The replay
uses a sample of the simulation's recorded impact coordinates and applies no
damage. Native defender setup deploys up to four ships, and the replay's
total number of visible impacts is capped at three times the number of
defenders. The raid log distinguishes visible, unshown and unresolved hits.
The incoming sightline resolves the first exposed module, then HE damage
falls off from that point and armour reduces exposure behind it. Any real
module can be hit, including bridges, engines and other critical parts.
Zero-HP parts use silent native removal; a critical loss can kill a ship.
No player ship is deployed, and no campaign missile is spawned or consumed.
The tactical mix is deliberately much louder, with frequent stock samples
whose transients are already clipped, mixed with metal impacts and cracks.

After the replay, the stock BLACK overlay fades the scene to black over one
second, then the video task presents a fully black frame before the game thread
transfers precomputed final HP and part losses to the actual defenders. The
cover remains through teardown. The native kill-confirmation cue is suppressed
at all six queue sites (`Body_DieExplode` and the five `ShipStatusCheck` "lost
ship" branches) and stays suppressed for five seconds after the native return,
because status checks can run during the handoff. Native missile-scene teardown
returns survivors to the campaign. The raid disables unloading on the linked
garrison to preserve full ship records. No damage is queued
for normal enemy combat spawns. `shelling simulation complete before replay` records
the damage totals, `shelling campaign damage retained` records the unload opt-out, and
`shelling raid native return completed` records the teardown and the number of
suppressed kill cues. The log includes `shelling replay ended; fading to black`,
`shelling blackout reached opaque` with the observed duration and alpha,
`shelling black frame presented`, defender IDs, driver state, HP and part
counts before the replay and around damage application. `ALLY SPAWN` includes
absolute part counts and HP; a template can lose parts and still show 100% HP.

## Barrage behaviour

- One barrage runs at a time. A second fleet's **FIRE** reports that another
  barrage is under way.
- A barrage does not alter the firing fleet's speed, route, move orders or
  altitude. The user can maneuver throughout firing and while the last
  rounds are airborne.
- Barrage time uses the campaign clock (`hourCount_`, `0x143a119c8`), so time
  compression advances the countdown. Leaving the map early or aborting discards
  the barrage; no delayed normal-combat hit remains.
- FIRE starts the turret servo and localized crew acknowledgement together.
  Once both enabled cues finish, one random `shell_out_big_far_01..04` report
  plays per working barrel (one for the 1-180, two for the Sarmat, and six for
  the Squall) in quick succession. Bursts of two to five big-far cannon reports
  with stock clipped transients follow every 6.8–7.8 seconds while the firing
  fleet is selected. CCU uses its own stock sound cues independently of
  the Allies `ally_sounds` setting.
- Map visualization uses the native `Gfx_DrawWorldLine` primitive on the M11
  seam, not text glyphs. Each visible shell is a bright short tracer with a
  cross-shaped centre. Dots keep travelling when another fleet is selected and
  follow the battery's launch cadence. At high cadence up to 24 airborne rounds
  are sampled by a fixed per-barrage stride, so a dot keeps its identity through
  impact instead of jumping as the sampling window moves. Small pale bursts are
  misses and larger orange bursts are effective hits. The visual fade does not
  hold up tactical resolution.

## Native integration (Steam 1.163)

- `PLAYER_ESCADRA` at `0x143a13eb0` is the fleet used by native map orders.
  Read the `Escadra` ship-property list at `+0x100` and use
  `GetPropertyLinkedObject` (`0x14021b070`), as the game does in
  `Escadra_FindLandingShipNode` (`0x1400f5b40`). Look up the equipped guns
  through the ship's two gun groups and `GetStats` (`0x140281e00`).
  `FireGun` (`0x140032f4a`) reads the mount's rounds per minute at
  `Stats+0x150`; `Part_Tick` (`0x1400553b5`–`0x140055516`) uses the
  magazine and reload time to load rounds one at a time. The stock Squall
  definition is `MDL_CANNON_305_2`, so eligibility uses calibre, not the ID.
- `Location_CreateMapGuiNode` (`0x1400ea6e0`) puts the `Location` in
  `MAP_LOCATION`'s `Gui::gui_l1`. The preview takes only the clicked
  coordinates and never keeps that game's pointer between callbacks.
- Before staging a raid, shelling calls the stock fleet expansion routine
  (`0x1400f9460`, called by `UpdateStrategicFleet` at `0x14011471e`) for
  defenders with a negative ship load time (`Creature+0xc80`). Without it,
  compact three-part campaign records can deploy as ships and disappear on
  native return before any shell hits. The mod checks that compact designs
  exist, then re-reads the fleet and refuses to stage unresolved or still-compact
  defenders. `shelling defenders prepared` logs the part counts before loading.
- Shelling stages prepared defender fleet links in native side A and leaves
  side B empty, using the same Link construction and vector growth as
  native map engagements. `SetLevelSeriaToLoad_` loads `Missile.seria`. The
  shelling-only branches at `0x1402d4ce9` and `0x1402d4f93` skip the B[0] missile
  creation/count reset and restitution.
  The setup (`0x1402d45c0`) and teardown (`0x1402d4f20`) otherwise remain native.
  Teardown moves surviving creatures back into the campaign and recomputes fleet
  state. The normal Combat spawners are untouched.
- `UpdateStrategicFleet` calls its unload helper at `0x1401126ec`. When
  `Escadra+0x52c` is enabled and the unload deadline has expired, it can replace
  expanded ships with a compact design (`0x140111f3e`); the gate checks the
  timed `m_damageCounter`, not each part's remaining HP. The raid clears the
  native `canBeUnloaded` flag only on a garrison that takes damage, through the
   ship's resolved fleet pointer at `Creature+0xa70`, as read directly by
   native extraction at `0x14020f3d3`. That flag is serialized by `Escadra_RegisterProperties`
  (`0x14010085b`) and keeps the whole fleet resident, trading memory for
  preserved ship state without a second damage path.
- The level is a host for the native encounter contract, not the owner of
  the raid presentation. There is no `Shelling.seria`: cloning the asset
  would still run `SCR_MISSILE`, and a new script would need its own setup and
  return path. The raid snapshots defender hull bounds and samples terrain
  with `Landscape_GroundHeightAt` (`0x1402ee0c0`). At the battle renderer
  dispatch (`0x140300d7d`), it temporarily overrides screen target, zoom and
  shake for the full draw and restores stock values afterward. Incoming
  tracers invert that same projection to enter from above the viewport
  and draw before the stock battle overlay flush (`0x140309408`), not through
  the player target-box branch.
- `Gui_BlackoutOverlay_BeginClear` (`0x1402130b0`) and `_BeginOpaque`
  (`0x140213fc0`) use the singleton at `0x143a13b08`. Logic id 10000 handles the
  element in `creatureLogic`: state 0 ramps alpha (`+0x8f8`) toward the target
  at `+0x3d4` by `dt / +0x3d0` per tick, state 1 holds that target, state 2
  ramps alpha down by `dt / +0x3d0`, and state 3 is inert. The raid holds the
  `BLACK` cover opaque (`state 3`, alpha 1) across the level load, fades it out
  over 0.7 s when the wind cue plays, and later fades it back in over 1 s before
  the damage handoff. The fade-in creates a fresh element with
  `Gui_CreateBlackOverlay` (`0x1401ca120`, mode 1) because the level-load
  instance can be unlinked before the replay ends; the returned pointer is
  polled for alpha instead of the singleton. This hides the visible level
  assembly without a new asset.
- At a tactical strike, `FX_SpawnExplosionDebris` (`0x1401c7270`) creates
  the stock particles and transient effect light. Shelling does not call
  `Body_Explode`, create a damaging projectile, or run a native blast pass.
  The FX routine's audio tail (`0x1401c937f`) is skipped only during shelling's
  synchronous invocation; ordinary explosions still use stock audio.
- Vanilla 180 mm HE shells (`FireShell` case `0x19`) spawn with
  `lightPower = 10000`, the warm light colour `(1.0, 0.9843, 0.8235)` and a
  fast fade (`Body_InitializeCannonShell`, `0x140052a80`). The spectator
  tracer layers the same palette as translucent wide lines over a bright
  core; no damaging projectile is created.
- The raid's HE approximation uses the rule documented in
  `hf_notes/docs/findings/combat/explosion-damage-1151-vs-1163.md` and
  `hf_notes/docs/findings/combat/blast-and-penetration.md`: stock 180 mm
  HE has explosive power 82, radius `sqrt(82)`, and damage
  `floor(10 * sqrt(82) * (1 - d/R)^2 * visibility)` inside that radius,
  where `d` is distance to the part's edge. Armour takes one fifth and
  shadows the angular span it covers, processed nearest first. The raid uses
  native module centres and bounding radii, not exact collision fixtures,
  and caps the sum of requested HP loss at 290 per representative hit.
  No AP chain, incendiary dose or secondary native blast is invoked.
- Shell tracers call `Gfx_DrawWorldLine` (`0x140248ba0`) with world-space
  endpoints. The native function performs campaign projection, screen clipping
  and HGE quad submission, avoiding the invisible font-overlay path.
- `Escadra` and `Location` share the `Mark` position at `+0x150/+0x154`.
  `Escadra_ApplyMapModeFields` (`0x1400f92b0`) sets the current altitude
  at `+0x168` to 9000; `+0x368` is the requested altitude, not the current one.
- The map renderer's flush call (`0x14030dae4` → `0x140083b50`, M11) observes
  the firing fleet's position after its movement update. It follows the
  original fleet identity through selection changes, interpolates new
  tracer launches between observations, and never changes an airborne
  tracer's launch point.
- The preview adds no hook sites. It uses the ally DLL's existing game-thread
  GUI input and click dispatch, with bounded fleet, gun and GUI-list reads.
  The native control lives on stage 67, after the stage 65/66 cockpit panels,
  rather than on the default stage 60 shared with map markers.

## Next gameplay slice

The current resolver keys on the city coordinate and the hostile role-2
garrison within 5 units. Follow-ups: resolve against the `Location` identity
instead of coordinates, add the city-service refusal, and trigger the surviving
garrison's alarm.

The native strategic blast wrapper (`Escadra_ApplyBlastDamage`, `0x1400f0cc0`)
is not used. The tactical raid uses native missile encounter aftermath instead.

For refueling, stock station transfer runs separately from the fuel purchase
screen. Zeroing stock would remove fuel, not establish service refusal, and
wouldn't prove that another fill path cannot replenish it. The scripted
`RUINED_FUEL` action is not a service ban: it clears `Location+0x305` at
`0x1400b830b` after the player salvages fuel. The saved `m_ruined_fuel` byte
controls whether ruined-city dialogue offers `RUINED_CITY_ORDER_FUEL`
(`0x1400a9d93`), not whether a normal station sells or transfers fuel.
The first refusal implementation needs one per-city policy checked at the
native buy path (`FUEL_BUY_SLIDER`, `0x140137140`) and the docked player
transfer (`0x14011525c`), leaving transfer from other sources and unhit
cities alone.

## Manual checks

- With the option off, or on a fleet with only 130 mm or 203 mm guns, no
  preview control should appear. Select a different fleet with 180 mm guns
  and check that it appears.
- Left-click a city in preview mode and cycle the duration. Rounds and expected
  hits should increase while ETA stays the same and matches the first map burst.
  Change the distance and check a target just inside and outside range,
  landed and airborne.
- Press **FIRE** while the fleet is moving. Change its orders or altitude
  and confirm they still work. The countdown must advance with time
  compression and the gunfire must stop when another fleet is selected.
  **ABORT** must cancel the barrage without changing the fleet's orders.
- Press **FIRE** and keep both the fleet and target in view. Bright native-line
  shell tracers must visibly travel the whole route, not sit at the muzzle or
  city. Pale bursts are misses; orange bursts are effective hits. Turn the
  battery and select another fleet: new tracers should follow the original
  battery, and airborne tracers should not jump.
- On a backup campaign save, let a barrage finish against a hostile garrison.
  Verify the camera holds the whole group near the bottom and neither chases
  impacts nor shakes. Check the wind-only opening, the lulls between salvos,
  and that rounds enter from above and visibly end in
  stock flashes, smoke and flying
  debris on hulls and dirt, and that it then returns to the map without an
  ordinary battle. Compare a single mount and a heavy battery: the latter
  should have more ground bursts, denser groups and a less uniform sound mix.
  Check the preparation, impact, unload opt-out and native-return logs.
  Re-shell the same garrison; damaged survivors should not need expansion.
  Save/reload, then enter ordinary combat and confirm the same damage and no
  extra hit at spawn. Also check a city with no garrison and a normal missile
  attack, neither of which should open a shelling raid.
- Compare equally sized groups of 1-180s, Sarmats and Squalls. The installed
  stock data gives 12, 16 and 65 rounds per mount respectively in a one-minute
  preview (a round due exactly at the end is excluded); the shortest barrage is
  ten minutes.
- Cancel, switch fleets, open a modal, and leave and re-enter the map. Check
  that targeting does not leak into vanilla map orders or other screens.
