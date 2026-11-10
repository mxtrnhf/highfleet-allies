# Changelog

## 0.6.2 — 2026-11-10

- Validate driver and obstacle-scan timing groups independently. The occupied
  driver site remains untouched, while the two original, disjoint scan CALLs can
  be measured. Both scan sites must match before either is prepared. Unexpected
  preparation or core installation failures still abort the uninstalled batch.
- Driver/scan CSV availability is tracked separately: absent coverage stays
  blank, measured zero stays zero. Core ai_us remains inclusive of both native
  AI and installed mod hooks. Scan timing excludes the previous obstacle-list
  free loop and must not be added to inclusive ai_us.
- Sidecars record prepared AI span coverage and the ally configuration read at
  initialization, making panels-on/off runs distinguishable. No panel, AI
  decision/cadence, render, physics or memory-guard changes.

## 0.6.1 — 2026-11-10

- Refuse conflicting optional AI span sites without aborting core frame/task
  profiling. Validate all three sites before preparing any optional patch;
  preserve guards and the existing all-or-nothing span group. Missing driver
  and scan timings remain blank, not zero. Unexpected preparation or core
  installation failures still fail closed.
- Bring the existing profiler source from benchmark commit aa9c5dd onto main;
  no benchmark results or unrelated branch changes imported. No new hooks,
  gameplay/cadence changes or memory-guard reductions.

## 0.6.0 — 2026-11-09

- Add native AI attribution spans (`docs/ai-plateau-attribution-2026-11-09.md`):
  wrap the `aiShipDriver` call in the `SomethingAITask` loop (0x14001123a) and
  both `AiShip_ScanObstacles` call sites (AiShipTick 0x140064e6f,
  AiShip_SteerController 0x14006cf83) with entry/exit QPC timing. Call sites are
  wrapped rather than function entries so every argument register and stack
  argument reaches the original callee untouched. New CSV columns:
  `ai_driver_us_total`, `ai_scan_us_total`, `ai_driver_calls_total`,
  `ai_scan_calls_total` — per frame, alongside per-`ai_us` share.
- The five `ai_*_us_total` / `ai_*_calls_total` hf_ai columns are now per-frame
  totals (swap-and-reset each row) instead of process-cumulative counters, so a
  telemetry build of hf_ai (feature `telemetry`) finally attributes the mod's
  hooks inside `ai_us`/`game_us` per frame.
- New config key `ai_spans` (default true); set false to measure the span
  hooks' own overhead. Byte guards fail closed: any call-site mismatch leaves
  the native code unpatched and the columns empty.
- No behaviour change to the game; the timing hooks only read QPC and add
  atomics.

## 0.5.2 — 2026-11-08

- Add `heuristic_p25_us` and `heuristic_p50_us` CSV fields read from the game's
  own 20-entry work-time ring (`DAT_143a1c450`/`458`), so the stock pacing
  sample can be compared with `frame_native_us`.
- No change to the frame-skip decision or native code.

## 0.5.1 — 2026-11-08

- Add `render_calls` and `render_false` CSV counters to distinguish a frame-skip
  hook that never ran from a stock pacing policy that never chose to skip.
- No change to the frame-skip decision or any native code.

## 0.5.0 — 2026-11-08

- Add `frame_skip`: wrap `SomethingVideoTask` `0x140316e20`, read the pacing
  boolean that `_FrameFunc` already passes in `ECX`, and set the game's existing
  one-shot skip flag `DAT_143a11963` when the boolean is false. The native video
  task then takes its own epilogue path; simulation/input/AI/game tasks are not
  skipped.
- Add the entry-prologue byte guard and a Win64-alignment-preserving trampoline.
- Add `render_skip_frames` to the profiling CSV counter set.
- Default off; installed research config enables it for the first A/B. In-game
  visual/pacing validation remains the user's step.

## 0.4.0 — 2026-11-08

- Add a real vanilla-engine optimization for KAZ/active-protection target scans:
  cheap scalar/stats rejection before native virtual/distance calls, one stats
  lookup per candidate, original live array order and first-target semantics.
- Keep the native ammunition/cooldown update, second detonation scan, effects and
  RNG untouched. No cross-frame body cache or global array substitution.
- Add independent `kaz_fast_scan` switch (default true under the opt-in master),
  full target-loop/helper byte guards, and profile activation/rejection counters.
- Includes the v0.3.0 profiling implementation; Windows build/check only. In-game
  equivalence, activation and FPS improvement remain unmeasured until user run.

## 0.3.0 — 2026-11-08

- Add startup-only frame wrapper preserving the native quit result, fresh native
  task timings, simulation progress, bounded scalar world counts and activation counters.
- Add independent opt-in profiling, bounded asynchronous CSV output and a
  versioned optional hf_ai span sink; no AI/gameplay/tick changes.
- Add offline native-profile CSV reporting. Loop intervals are not presented FPS;
  native task samples include mods and remain blank when stale/unavailable.
- Build/check only; in-game validation must be performed by the user.
## 0.2.0 — 2026-11-08

- Add callback-scoped shared fixture discovery and balanced spatial hierarchy
  for both native incoming-warning ray fans, preserving native intersection code
  and candidate ordering.
- Add per-fan first-ray native-result auditing and native-loop fallbacks.
- Retain font batching; no AI/physics/damage/tick-frequency changes.
- Add `hud_spatial_queries` and `verify_queries` settings (both true when the
  master switch is enabled), standalone spatial benchmark and detailed RE notes.
- User reports v0.1.0 crash-free startup; v0.2.0 awaits user in-game validation.

## 0.1.0 — 2026-11-08

- Independent, experimental Steam 1.163 engine mod: batch font recoloring and
  skip writes only when all four actual glyph-corner colors already match.
- No glyph cache, gameplay changes, persistent native pointers, or tick changes.
- Default-off strict config; version/base/full-function byte guards.
- Standalone original-instruction microbenchmark; no whole-game speedup claim.
