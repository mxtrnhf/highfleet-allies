# hf_ally v0.16.2 performance changes

## Design: one callback-local view, multiple consumers

The repeated work came partly from treating each consumer as an independent native-memory
discovery task. Instead, share a validated view within a callback. Do not retain native object
identities or memory-readability decisions across frames: destruction, scene changes, RETRY and
pointer reuse make those caches unsafe without a stronger native lifetime contract.

Screen lifetime is also a state transition, not a task to repeat every combat tick. Previously,
every armed battle tick with a nonempty roster called `hide_toggles`, which scanned the full GUI
list twice even after precombat was already closed. Cleanup now runs once on exit, shares one
ownership scan between plane/ally controls, and retries from the input/battle tick only while a
scan was incomplete. Normal combat after successful cleanup does no GUI cleanup scans at all.
Retained pointers are still checked against current list membership before they are touched.

### Ally panels

Both panels now share one discovery pass over the 60 render-body layers and one over HUD-marker
layers 1..39. Owner reads are guarded and memoized within discovery only. Ordered body vectors
preserve native traversal order; hash sets handle membership rather than repeated linear searches.
Sprite-layer iteration is allocation-free. The native panel still renders once per visible ally,
and temporary markers are still restored after each call.

For two allies, mod-side global-table discovery is reduced from two passes per table to one.
For one ally, table-pass count is unchanged; duplicate owner queries and linear membership searches
are removed. These are structural savings, not measured frame-time improvements. Native rendering,
texture compositing and the game's extra ship/physics/projectile workload remain.

### Aircraft

The active-body view is a bounded streaming iterator, not a `Vec` of every active body's scalar
fields. Each hunt still checks the same bodies in the same order and invokes the native active
predicate. No cross-aircraft/frame target cache, changed scan interval, simplified active-state
test, target-priority change or nearest-distance approximation is introduced.

Diagnostic census scans, periodic AA watch messages and delayed loadout checks now require
`logging.debug_log`. AA speed scaling, launch tracking, inventory charging, gunpods and payload
exhaustion remain independent of logging.

### Precombat and debug

Each precombat tick uses the same complete GUI scan for lifetime and layout decisions; an
incomplete scan still cannot establish screen absence. Fixed-name checks avoid constructing a
`String` for every active GUI element. GUI labels and card following retain their frame frequency.

Debug probes validate every page of the bounded creature-link span once per callback, then borrow
only its scalar pointer values. Cached ally, enemy and target identities still require current-list
membership before object fields are inspected. Object-field reads remain guarded. No owning native
container or Rust reference into game memory is constructed. Weapon sampling remains per tick;
its printed diagnostics retain the original timing. Snapshot reasons use deferred formatting when
debug is off, and the watchdog formats its heartbeat at 2 Hz rather than 10 Hz.

The megalog intentionally remains synchronous/unbuffered: batching it would change how much
diagnostic evidence survives a crash. Overlay visuals and their render frequency are unchanged.

## Native evidence and rejected shortcuts

Existing notes consulted:

- `hf_notes/docs/rendering/battle-ship-panels.md`, including the v0.14.9 failed native-initialization
  experiment and v0.14.10 correction.
- `hf_notes/docs/findings/aircraft/carrier-plane-loadouts.md`, sections 18c and 18f.

Live Ghidra (HighfleetServer, Steam 1.163) rechecked on 2026-11-08:

- `Body_UpdateHudMarkers` (`0x14002d7b0`): active/opacity/init gates, sprite-vector iteration,
  inherited stage, four-layer sprite range, scale and child recursion. Simply calling native
  initialization on the ally was already disproven by the recorded in-game experiment. No state,
  opacity or initialization flags are changed by this optimization.
- `Node_IsStateActive` (`0x140005e60`): node state >= 2. The aircraft scan nevertheless retains its
  virtual predicate instead of replacing all native implementations with this scalar check.
- `Hud_DrawChildMarker` (`0x1402e08b0`): type-2-only draw, active predicate and sprite-index bounds.
- `AircraftLogic_LaunchAAMissile` (`0x14000f5b0`): target logic IDs, missile count/range/timer,
  friendly-ship bubble and alignment gates, native target Link and ammunition consumption. This
  launch helper does not replace the mod's hostile-target selection. The notes also establish that
  native mode-2 target selection has no side filter, so delegating selection to it is not equivalent.

Ghidra was read-only; no scripts, database edits or saves were needed.

## Validation boundary

Validation performed on 2026-11-08:

- `cargo xwin build --release --locked --no-default-features --target x86_64-pc-windows-msvc`: passed.
- The same release build with `--features megalog`: passed.
- `cargo xwin clippy --release --locked --target x86_64-pc-windows-msvc` with no default features
  and with `megalog`: both completed, 19 existing warnings each. An exported v0.16.1 baseline
  produced 23 warnings with the same toolchain; four modulo suggestions disappeared in touched
  diagnostic code. No lint suppressions or dependency changes were added. Remaining categories:
  formatting/style suggestions, documentation-list indentation, derivable Default, type complexity,
  map_or simplifications and C-string literal suggestions. Representative locations are
  `debug_overlay.rs:50`, `ally_hud.rs:9`, the `WATCH` type in `plane_loadouts.rs`, and the config
  Default implementation in `lib.rs`. Unrelated cleanup was not performed.
- `git diff --check`: passed.
- `cargo fmt --check`: fails on repository-wide formatting differences, also reproduced on the
  untouched v0.16.1 export (267 diff sections). No crate-wide formatting rewrite was applied.

The v0.16.2 build was installed in Steam and included in the 1.0.0 release pack. Validation logs
and the baseline export are in `/tmp/opencode/ally-performance-2026-11-08/`.

Cross-builds and static checks cannot validate native hook execution, frame timing or appearance.
The maintainer ran the current build in game on 2026-11-08 and confirmed that the mod works. The
detailed stress checklist was not individually logged: two allies with large hulls, damage and
destruction, control handoffs, retreat, RETRY, precombat shuffles and K-13 acquisition with debug
both off and on remain useful regression scenarios. No regression tests were added (repository
policy requires explicit permission). No measured FPS or millisecond claim is made.
