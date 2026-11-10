# hf_perf — vanilla engine optimization and frame profiling

Standalone **Steam 1.163 / Modloader v4** DLL. No dependency on ally/AI/QoL.
Current version **v0.6.2** contains:

| Feature | Purpose |
|---|---|
| Frame-skip pacing restoration | Reads the pacing boolean that `_FrameFunc` already passes and sets the game's own one-shot skip flag `DAT_143a11963`; simulation/input/AI/game tasks are not skipped |
| Native KAZ target-scan fast path | Reject irrelevant bodies before virtual/distance calls and avoid repeated stats lookups; retain native first-target order and all firing/detonation code |
| Shared HUD warning-ray spatial queries | Existing v0.2.0 conditional optimization |
| Batched font recoloring | Existing v0.1.0 optimization |
| Frame/task profiler | Whole native frame, seven task timers, simulation progress, workload and activation counters, bounded asynchronous CSV |
| Optional hf_ai span sink | Inclusive/nested measurements from a separately instrumented AI build; not Rust-only time |

**v0.6.1 core initialization was confirmed in game. v0.6.2 scan spans need runtime validation.**
The user performs runtime testing; the agent does not launch or attach to the game.

- [Frame-skip restoration and validation](../docs/frame-skip.md)
- [KAZ optimization and exact native boundaries](../docs/kaz-fast-scan.md)
- [Profiling configuration, CSV fields and analysis](../docs/frame-profiling.md)
- [HUD spatial architecture](../docs/hud-spatial-queries.md)
- [Earlier research/font baseline](../docs/vanilla-performance.md)

Build:

```bash
cargo xwin build --release --target x86_64-pc-windows-msvc --locked
```

Install as `Modloader/mods/hf_perf_v0.6.2.dll`; keep only one active hf_perf DLL.
Tracked/missing configuration defaults to fully disabled. The installed research
config explicitly enables the existing optimizations, KAZ fast scan, frame skip and
profiling. Invalid config, unexpected version/base, or native byte mismatch fails
closed.

### Optional AI-hook compatibility

With `profiling:true`, `ai_spans:true` requests three additional CALL-site
timers. v0.6.2 admits the driver and the two-scan group independently. If the
driver site differs (including hf_ai's tick-skip at `0x14001123a`), it stays
untouched and driver columns stay blank. Both scan CALLs must match before
either is wrapped; a mismatch leaves scan columns blank instead. Core profiling
continues either way, including the native inclusive `ai_us` timer.
Blank means unavailable, not zero cost. Scan time nests inside ai_us, covers
the two AiShip_ScanObstacles calls and excludes the previous-list free loop.
Preparation/allocation failures and core byte mismatches still fail closed.

For profiling without activating legacy engine/HUD/render optimizations use
`enabled:false, profiling:true`. This is the deployed follow-up configuration;
the other feature settings are preserved but gated off. No gameplay or AI
cadence change is needed. The CSV output directory is retained from the live
config. Check the startup log for `profile CSV prepared` and final
`profiling=true`; runtime initialization still requires a user game run.

For the panels-off AI measurement, preserve `ally_panels:false` in ally.json.
See [the capture procedure](../docs/native-ai-attribution-2026-11-10.md#next-capture).
The profiler sidecar records the initialization-time ally config and prepared
driver/scan coverage; installation success must still be checked in the log.

Source was selectively imported from benchmark commit `aa9c5dd`; results and
unrelated benchmark-branch edits were not imported. Historical documentation
and analysis tooling referenced below remain in the benchmark worktree.

Independent switches under `enabled`: `hud_spatial_queries`, `verify_queries`,
`kaz_fast_scan`, and `frame_skip`. `profiling` and `profile_output_dir` work even
with `enabled:false`.
An absolute Windows/Wine output path such as `Z:\\home\\...\\captures` is required.
Restart after changes; never unload/inject during play.

Analyze completed CSV from the repository root:

```bash
python3 tools/hf_bench.py native --input captures/hf_perf_v0.5.2_*.csv \
  --label kaz-fast --scenario heavy-battle-v1 --conditions fixed-v1 \
  --output captures/kaz-fast.native.json
```

Shell wildcard expansion is inappropriate if it selects multiple files; supply the
one completed capture explicitly. See the profiling guide for warmup/duration,
repeated comparison, and measurement limits.

MIT, see [LICENSE](../LICENSE). Thanks to logdot for
[Highfleet-Modloader](https://github.com/logdot/Highfleet-Modloader),
[patchy-rs](https://github.com/logdot/patchy-rs),
[highfleet-qol](https://github.com/logdot/highfleet-qol) and
[highfleet-rs](https://github.com/logdot/highfleet-rs).
Native Node/Body offsets use highfleet-rs pinned to full reviewed revision
`05c62078c6f896c141713a68b3991663e121b7bf`, with compile-time assertions; game
ownership remains native. Uncovered KAZ/profiler fields use documented local
scalar offsets and do not create owning native layout copies.
