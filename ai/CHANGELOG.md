# hf_ai changelog

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
