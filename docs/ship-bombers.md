# Ally ship bombing runs (Steam 1.163)

Enabled by default in the 1.0.0 pack (`"ally_bombers": true` in
`Modloader/config/ally.json`). Set it to `false` to disable. This is
**FAB1000-carrying ships**, not carrier aircraft or their FAB loadout buttons.
The game classifies carried ship bombs through its `+0x1438` vector, not a ship-name rule.

## Behaviour

- Only tracked mod-deployed allies, AI-controlled and in airborne engage mode.
- Start with the existing target; it must be a live enemy, landed (`+0xb48 == 3`), and
  not capturable (`+0xc19 == 0`, the same flag used by `spare_prizes`). This
  restriction is unconditional, even when prize sparing is disabled.
- Dive immediately toward the target, without alignment or braking phases. Double
  the native desired-speed target. Command both `0x66`/`0x67` engine banks continuously
  along the downward bombing course: 0.5 seconds at Shift-like strength `2.0`, then 1.0
  second at normal strength `1.0`, repeating. Normal periods are not engine-off periods.
  Boost pulses are suppressed when the native max engine temperature reaches 350 or an
  engine lock is reported; normal thrust remains commanded. Native ignition is 400 and
  lockout 500. Pull-out acceleration uses normal rather than boosted lift when heat-limited.
  This reduces boost heating but cannot guarantee continuous physical force from damaged,
  unpowered, fuel-starved or already-locked engines.
- Apply player-style held movement commands while lining up the **native predicted impact**.
  Both swivel commands (0x64/0x65, angle +4) and thrust commands (0x66/0x67, angle +0x10)
  receive the same world direction every tick, as Player_SendThrustCommands does. Sideways
  corrections blend into the held direction like strafing; no global mouse/key state is touched.
  There is no minimum-speed, fixed-altitude or six-second approach gate.
- Descent is **governed by the height still available to pull out**: desired downward speed is
  `0.7 * safe_descent(height above clearance, recovery acceleration)`. Without this the ship
  reached ~250 u/s, the conservative pull-out distance exceeded the remaining height, and the
  run aborted before the aim could converge (observed 2026-11-01 01:08 run: four aborts with
  predicted misses still 130-300). While more than 220 units away in X from the target, the
  dive holds altitude so the lateral aim can line up first; it is not a separate align phase.
  The governor caps at 220 u/s, uses 85% of the achievable descent and 80% of available lift
  (not half), so a genuine fast dive is still recoverable; hull margin is 80 units.
- Lateral aiming uses the signed native miss at 10 Hz: proportional gain 1.2 while the miss is
  more than four tolerances away, then 0.5 with 0.75 s derivative damping near the target to
  prevent overshoot.
- **Release ceiling: 1000 units above the target.** It is an upper bound only. The bomb also
  needs real downward speed (`max(80, 0.6 * achievability)` u/s), because the native predictor's
  10-second horizon otherwise reports a "hit" while the ship is nearly hovering, which produced
  slow bombs. A run that **starts already below 1000** (possible at low spawn) needs no special
  handling: it satisfies the ceiling immediately, skips the lateral altitude-hold and simply
  builds speed and releases. Native no-prediction (`FLT_MAX`) results are ignored.
  Begin a **whole-payload salvo** inside that window when the predicted absolute horizontal
  miss is within the target's own bounding radius. The native "hit" flag is accepted too, but is
  not required: it also encodes the target's half-height band and stayed false at a 35.5-unit
  miss against a 37.2-unit tolerance (2026-11-01 01:22 log).
  After the first aimed release, detach every remaining bomb on the same target at 0.05
  game-second intervals (rounded up to the next AI tick); do not re-gate every bomb on an
  accuracy window. Native detach/ownership and landed/non-prize checks still apply for each.
  There is no multi-target switching.
- After the final bomb, continue horizontally in the bomber's
  current run direction while pulling up, preserving the sign of lateral velocity rather than
  reversing based on the player's position. Both servo/force command pairs receive the up/forward
  vector, with strength 2.0 while cool or 1.0 while heat-limited. Escape lasts at least 1.5 seconds
  and returns to normal AI only when vertical velocity is no longer downward and hull clearance
  is safe. Direction is selected once at salvo completion. Recovery uses cached
  scalar target geometry, not a live target dependency, so destroying the target cannot
  cancel the escape. This is not a general collision-avoidance guarantee.
- While any bombs remain, pulse every attached flare launcher through the native release
  routine every 0.5 game seconds, including during pull-out. Native ammo and launcher
  cooldown gates still apply; ships without launchers do nothing. Flare pulses stop after
  the last bomb.
- Block native ship-missile launches while this ship has a current bomber-owned course,
  including Recover. Guard the native `0x14002ff60` entry so `hf_ai`'s burst wrapper cannot
  bypass it. Other ships and player-controlled ships retain native launches.
- Armed bombing and recovery courses take priority over `hf_ai` missile evasion. Steering
  is applied after `hf_ai`, and native engine commands are restored after its engine hook
  before bomber thrust is applied. This prevents missile steering, panic boost and barrel
  rolls from cancelling the run. No dependency on `hf_ai` is needed.
- Ground safety uses downward speed, conservative recovery acceleration, hull clearance
  and 0.75 seconds of response time to enter Recover before the dive reaches terrain.
  Unsafe clearance aborts with the remaining payload instead of chasing a shot down to hull
  level. Because steering applies that guard before the weapon hook runs, a shot that becomes
  valid on the abort tick would be lost: release is still accepted for 0.5 s after the dive
  turns into Recover while the ship is moving down, and that salvo keeps the recovery course
  instead of re-diving. No blind or forced inaccurate salvo is used.
  This is predictive pull-out, not a braking/pass phase. Recovery steering also has priority
  over missile evasion. Insufficient net lift prevents starting. No bomb is released at an
  airborne/trader target or during retreat/promotion.

Release uses the game's `Ballistics_PredictHit`, not a separate approximate
trajectory formula. Its input is the bomb's world position plus radius/5-unit
release offset and the ship's velocity plus the native local-down ejection impulse
(profile `+0xac`). The native integrator includes gravity, drag, target motion
and nearby same-side ship obstruction, with a ten-second horizon. It is still
not terrain-aware or a guarantee against collisions/friendly fire. Native
obstacle avoidance still supplies the engine-command directions restored by the
post-`hf_ai` priority hook.

## Live RE evidence (2026-10-31 / 2026-11-01, read-only Ghidra)

Program: `/Highfleet.exe.unpacked.exe v1.163`, HighfleetServer. No scripts,
renames or transactions used. Existing research: `hf_notes/docs/structures.md`,
`findings/ai/ai-behaviour-states.md`, `findings/ai/ai-tick-patch-sites.md` and
`findings/player-thrust-keys-and-engine-heat.md`.

| Site / field | Verified contract |
|---|---|
| `0x140064a96`, 5 bytes `f3 45 0f 5c cc` | Immediately after `hf_ai`'s `0x140064a8d` waypoint hook. Save the current world XMM9/XMM10, replace them for an active bomber, reload them, repeat the earlier X store at `+0xbf0`, then replay `subss xmm9,xmm12`. Resume at `0x140064a9b`, whose native instruction stores Y at `+0xbf4`. Armed bombing/recovery therefore overrides missile evasion. |
| `0x140064f58`, 5 bytes `f3 0f 10 55 a0` | Native desired speed is in XMM12 after waypoint normalization and obstacle scanning. Preserve/replay `movss xmm2,[rbp-0x60]`, save XMM12 around the callback, double it only for an owned Dive course, restore it, and resume at `0x140064f5d`. Recover retains the native speed target. |
| `0x140065734`, 7 bytes `c7 45 20 00 00 00 00` | All four AI stack engine commands initialized. Pass RBP+0x28 (0x66); validate 0x64 at RBP-0x18, 0x65 at RBP+0x8, and 0x67 at RBP+0x48. Snapshot all 0x80 bytes. Write world angle atan2(course_x,-course_y) to the servo records' +4 AND force records' +0x10; force strength is +4. Dive/Salvo pulse 2.0/1.0; Recover uses 2.0. Heat limit overrides boosted strength to 1.0. Resume at 0x14006573b. |
| `0x140065750`, 5 bytes `49 8b 4c 38 08` | Header of the per-part command dispatch loop, not a once-per-ship site: `JC 0x140065750` at `0x1400657d3` returns here once per `Creature::parts` entry (`RDI` advances `0x18` per part; `MOV RDI,RBX` at `0x140065744` gives `RDI = 0` only on the first iteration). The trampoline now executes `test rdi,rdi; jnz <original instruction>` so parts after the first skip the callback entirely; only the first iteration restores the native stack command block saved at `0x140065734`, applies bomber thrust again, replays the replaced instruction and resumes at `0x140065755`. This removes later panic boost/spin changes for armed bombers only. Loop flags are safe: the first flag consumer after the head is `JZ 0x1400657a2` fed by `TEST RCX,RCX` at `0x140065755`, and the loop's `JC` is fed by its own `CMP` at `0x1400657d0`. |
| `0x14007bf15`, `e8 66 05 ff ff` | `AiWeaponController` calls `Creature_TryDropBomb(ship)` with RCX. Replace only this call; non-tracked ships still call the original routine at `0x14006c480`. |
| `0x14006c480` | Native release reads `+0x1438` Links, 0.75-second timer, target geometry and ballistic prediction. It does **not** enforce our landed/non-prize policy. Its loop also excludes the last selected bomb, so custom release does not depend on that loop. |
| `0x1401b5420(out, target, origin, velocity, side, ignored_ship)` | Native `Ballistics_PredictHit`; output is 8 bytes (flag at +0, signed horizontal miss at +4). Call ABI verified at `0x14006c96a`: RCX output, RDX target, R8 position, R9 velocity, stack faction/ignored ship. Integrates gravity 9.82 and drag 0.05 with `1 / *(float*)0x1403c0de4`, up to ten seconds. Native target dimensions are +0x1848/+0x184c; closest vertical crossing yields horizontal miss. The output flag alone is insufficient: also require the absolute miss within the release tolerance. Validate creature-vector stride/count and frame rate before invoking the native loops. |
| `0x14002e7d0(part)` | Native physical bomb detachment: native unlink/reparent to global node, root velocity, local-down impulse using profile `+0xac`, projectile metadata and logic 100. Called by both the player one-shot bomb control and native AI release. No Rust allocation is handed to the game. |
| `0x14002eed0(launcher)` | Native `FireFlareLauncher`, prologue verified. Called for each valid attached entry in the ship's `+0x1408..+0x1410` vector (stride 0x18, launcher at +8) every 0.5 seconds while bombs remain. Native loaded-count/rate limits remain in force; no player-only control byte or HUD branch is used. |
| `0x14002ff60`, 5 bytes `48 89 5c 24 10` | Native ship missile launch entry. Predicate borrows launcher's owner at +0xb8 and checks the current bomber course; the same entry guard also implements `ally_zenith_autofire` (AI-run allies hold their racks when it is false). Blocked launch returns AL=0 without native side effects. Other launches replay `mov [rsp+0x10],rbx` and resume at +5. Entry stack is aligned explicitly around the callback. |
| `0x140194de0(link, node)` | Existing ally `SET_LINK` / `SET_LINK_PROLOGUE` contract used by prize-safe target selection. Used only to pin the same bombing target against native AI retargeting, not multi-target selection. Never overwrite an owning native Link directly. |
| `0x140048fc0(node)` / `0x140048b60(node, out_vec)` | Root-part lookup / world-position output. Root lookup is reproduced as a bounded, page-checked Body-master walk; world-position output uses the native helper synchronously with a validated attached part. |
| Creature `+0x1850` | Bounding radius, from existing verified `CalculateStats` research. Used conservatively for clearance. |
| Creature `+0x15b8`, `+0x1664/+0x1668` | Total native mass and two engine-bank thrust sums; conservative net upward acceleration = half `(available_strength * sum_thrust / mass - 9.82)`, available strength 2 when cool / 1 when heat-limited. |
| Creature `+0x181c`, `+0x1828` | Native max engine temperature / any engine lock. Live CalculateStats disassembly 0x14008075d..0x140080785 reads engine +0x2ec into max +0x181c and +0x810 into +0x1828. Heat-limited boost threshold 350 (native ignition400, lockout500). |
| `0x140053690` / `0x140028360` | Live verified servo reads cmd+4, force reads cmd+0x10 and applies force only for positive dot product with actual engine direction. Prior bomber builds changed only force angles, leaving swivel direction under stock AI; mismatched commands could suppress physical thrust despite BOOST logs. |

Native function/hook bytes are checked before installing any hooks. Failure
skips this feature without partially installing it. Patch construction failures
propagate to the owning initialization session. Speed, thrust and release require
the same target/tick and an unchanged final bomber waypoint. The post-`hf_ai`
steering/engine hooks make that course authoritative while armed; stale or bypassed
decisions still fail closed. Scalar/vector accesses are bounded and page-checked;
no owning `Body`, `CVec`, strings or Links are copied.
Covered Body velocity/angle, Creature radius/mass/thrust/waypoint, bomb/flare
list and Link offsets come from the pinned `highfleet-rs` revision
`05c62078c6f896c141713a68b3991663e121b7bf`, also checked against current upstream.
Fields upstream does not model (engine temperature/lockout, bomb radius) stay
local with their raw offsets.

## Validation still required

Validation performed: locked Windows MSVC release cross-build, Clippy for the
default and `megalog` feature configurations, new-module rustfmt checks and
`git diff --check`. No warnings in the new modules. Existing crate warnings and
unrelated formatting differences remain; full `cargo fmt --check` fails on those.

No new regression tests were added (repository policy). Cross-builds cannot prove
native calling conventions, collision avoidance or release accuracy. Exercise:

1. One and several FAB1000s against a landed non-prize ship; verify continuous matched servo/force
   commands, native impact-window acquisition without a speed/altitude gate, all bombs at 0.05s minimum
   spacing (`t=` logged to milliseconds), and a forward/up pull-out preserving horizontal direction.
   With several landed enemies, verify the entire salvo remains on one target. Verify
   target destruction does not cancel escape; control returns only after >=1.5s and arrested descent
   with safe clearance. Verify unsafe approaches abort before hull contact. Check servo/force angles
   agree, boost is suppressed when temperature reaches350, and valid low-speed shots are not rejected.
   Observe heat and confirm the
   0.5s boost / 1.0s normal cycle never deliberately sets Dive bank thrust to zero.
2. Incoming missiles during Dive and Recover: bombing course/engine commands remain
   authoritative; a flare carrier emits `ALLY bomber FLARES` pulses every 0.5 seconds
   until the last bomb, and a flare-less carrier remains unaffected.
3. Target takes off during the dive; prize-only battle with prize sparing both on
   and off; no-bomb and damaged/low-thrust allies.
4. F8/fuel retreat, flagship handover, battle restart and a new battle.
5. `hf_ai` enabled/disabled and the feature flag false/missing. Confirm player bombs,
   enemy bombs, carrier loadouts and non-bomber missile evasion remain unchanged.

`ALLY bomber:` phase/release lines in the normal mod log identify each run.
`ALLY bomber release check:` reports downward speed, native hit flag, miss and tolerance;
`ALLY bomber FLARES:` reports actual native launcher pulses. Debug mode also logs
`ALLY bomber gate:` eligibility snapshots every three seconds.

## Installed-build diagnosis (2026-10-31)

`Highfleet_2026-10-31_23-01-07.log` loaded hf_ally build
`20261031-192904 UTC`; its serialized config contains no `ally_bombers` key and
there is no bomber-install line. The installed JSON already had `ally_bombers:
true`, but that DLL predates this feature. Rebuilding without installing the DLL
cannot change gameplay. Check both the build stamp and `ally ship bombers enabled`
after replacing the DLL and restarting the game.

`Highfleet_2026-10-31_23-54-09.log` then showed half-height dives but no drops:
missile evasion replaced the final waypoint and the same-tick release guard correctly
cancelled. One ship entered Recover only at `y=-39` with downward speed 85.4, was
overridden again, continued to `y=170`, and was destroyed near the ground. Moving
steering after `hf_ai`, restoring native engine commands after its engine hook, and
using speed-dependent predictive pull-out address those specific failures.
