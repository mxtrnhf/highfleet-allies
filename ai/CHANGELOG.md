# hf_ai changelog

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
