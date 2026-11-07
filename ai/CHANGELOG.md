# hf_ai changelog

## v0.10.19 — retire offensive early fire

- Remove the offensive early-fire subsystem entirely: the group-0 range-gate
  detour, `tactics.aim_fire_probability`, `tactics.aim_fire_range`, the hidden
  `tactics.aim_fire_s`, per-ship early-fire windows, gun-reach tracking and all
  related state.
- Old `aim_fire_*` keys are ignored by the permissive config parser; they no
  longer grant range permission or change native firing.
- Aim awareness, dive avoidance, missile defence, carrier groups and missile
  dumping are unchanged.
- The tracked preset now has 39 leaf settings (38 gameplay plus
  `logging.log_panic`).
- Cross-build only; no regression tests were authorized. In-game validation
  pending.

## v0.10.18 — retire the preventive flare curtain

- Remove the preventive flare-curtain subsystem entirely: launch scanning,
  curtain scheduling/math, per-ship curtain timers, rack accounting, curtain
  retreat steering, the curtain engine boost, and all `curtain_*` settings.
- Keep the reactive missile defence: panic flares, ordinary evasion, panic
  evasion, barrel rolls, per-missile reaction rolls and stray-missile evasion.
- Old `curtain_*` and `launch_flares_*` keys are ignored by the permissive
  config parser; they no longer enable or tune anything. Remove them from
  presets to avoid implying that they still work.
- The shared hf_ally debug-overlay value 5 is retained as a reserved retired
  mode so the existing cross-mod ABI does not shift.
- This is a real behaviour change, not a no-op: proactive pre-lock flare clouds
  and their incidental protection are gone. No regression tests were
  authorized. Cross-build only; in-game validation pending.

## v0.10.17 — rename preventive curtain keys

- Rename every preventive-curtain setting from `launch_flares_*` to
  `curtain_*` in the config struct, preset and documentation. This is a naming
  change only: defaults, ranges and behaviour are unchanged.
- Keep every old `launch_flares_*` spelling as a Serde alias, so existing
  `hf_ai.json` files continue to load without edits. Do not use both spellings
  for the same setting: duplicate fields fail parsing and the whole config falls
  back to defaults.
- The separate reactive panic-flare settings (`trigger_range`,
  `reaction_probability`, `burst_seconds`, etc.) are untouched; only the
  proactive curtain keys moved.
- The retired shelter keys remain retired under their historical
  `launch_flares_shelter*` names.
- Cross-build only; in-game validation pending.

## v0.10.16 — reaction probability is per missile

- `ai_flares.reaction_probability` is now rolled once when a missile first
  enters `trigger_range`, not on every 0.1 s threat scan. A failed roll records
  that missile body and ignores it while the threat picture remains; other
  missiles and later approaches after the picture clears roll separately. With
  `0.6`, roughly 60% of missiles trigger a reaction instead of nearly every
  missile reacting within a second.
- Panic timing, flare bursts, evasion, stray-missile handling and cooldowns are
  unchanged. No configuration or preset changes.
- Cross-build only; no regression tests were authorized. In-game validation
  pending.

## v0.10.15 — stop dive-denial chasing landed targets

- Dive denial now requires an airborne combat target (mode 1). Landed,
  taking-off or unreadable target modes cancel an existing dive as well as
  preventing a new one. Aim defense and early fire remain available.
- Tactical waypoint composition preserves the configured ground-clearance
  floor across the entire 400-unit course. Below it, tactics cannot add descent
  or weaken an upward native avoidance direction.
- Disable tactical boost/dash at or below that floor, restoring native engine
  braking instead of applying horizontal thrust while downward momentum persists.
  A failed waypoint composition no longer activates a tactical boost.
- No preset changes. This is a conservative fixed-reference guard, not terrain
  sampling or a guarantee against every stock-AI ground collision. Missile panic
  and flare-curtain maneuvers are unchanged. In-game validation pending.

## v0.10.14 — shared highfleet-rs layouts

- Add `highfleet-rs` at the merged `05c6207` revision. The Creature, Body,
  PartStats, AircraftLogic and Link offsets used by tactics, missile defense,
  missile dump and plane groups now come from its 1.163 types; a compile-time
  block keeps the raw offsets this mod depends on. Fields upstream does not
  model yet (node identity, gun reload/latch words, strategic TWR) stay local.
- No behaviour or configuration changes. Cross-build only; in-game verification
  pending.

## v0.10.13 — dodge and panic escapes actually dash

- Tactics waypoints are now 400 units (`tactics_math::WAYPOINT_LEN`). The stock AI tick's
  throttle is `clamp(sqrt(distance + 0.1) / 20, 0, 1)`, which saturates at 400, so the same
  dodge now asks for the full `25 * battle TWR` target speed instead of 71 % of it. This
  affects dive and aim dodges (and the panic waypoint, which reuses the tactic course length).
- Dive/aim boosts and the boosted panic evasion now dash instead of only raising the command
  strength. The engine hook points both banks' `0x66`/`0x67` force angles and the
  `0x64`/`0x65` servo angles along the manoeuvre's escape direction and floors the strength
  at the boost. The stock AI's velocity controller (`s * 25 * TWR`, with braking at the
  target speed) is bypassed for the duration, so the ship can actually outrun its own speed
  target the way a player holding Shift does. A boost of `1.0` still disables the boost and
  the dash; the flare-curtain retreat has no published direction and keeps the strength-only
  raise.
- `panic_boost` is only applied when it is above `1.0`, so `panic_boost: 1.0` really is off
  even when the shared engine hook is installed for tactics.
- Panic trace gains `ESCAPE` (`boost`, `dir`, strength before/after) next to `BOOST`/`SPIN`.

Validation: Windows release cross-build passes. In-game validation is required: check that
dive/aim dodges cover more ground and that static-engine ships still thrust sensibly (the
dash aims the force commands; only swivel engines physically turn to that angle). No new
regression tests were authorized.

## v0.10.12 — corrective follow-up (not published as a release)

- Replace forced player-fire-byte bursts with a byte-validated detour of the native
  main-gun (group 0) range gate at `0x14007ba84`. Offensive awareness rolls grant
  a one-second range permission; actual per-gun alignment, friendly-fire,
  ammunition and burst-start checks remain native. Group 1 is unchanged.
- Remove fire-byte writes and cleanup entirely. No pending native writes can be
  stranded on a clock reset, ship switch or reused address. Range permission
  validates native node/target identities, current AI control and recent steering.
- Allow defensive hostile-aim dodging without the defender's own living target;
  dive avoidance and offensive fire still require one.
- Prepare the shared engine-boost hook independently of flare-specific byte checks.
- Zero curtain-start probability no longer freezes an existing curtain's updates.
- Correct documentation: 49 total preset leaf settings (48 gameplay plus logging), one-time clamped ETA wait,
  pre-salvo reserve threshold (not a guaranteed 50% remainder), sampled fire-button
  hold-over, and native burst/friendly-fire limitations.

Validation: Windows release cross-build passes; supported-target Clippy completes
with 11 pre-existing style/doc warnings. `cargo fmt --check` still reports existing
crate-wide formatting differences; no unrelated formatting sweep was applied.
No new regression tests were authorized. Existing host
math harnesses contain zero tests and provide no behavioral coverage. In-game
combat validation is still required; installation is not gameplay validation.
