# Enemy-dependent slowdown — v0.1.5 follow-up

## Inputs and identity

- Main baseline2d9d2f5, installed `hf_native_v0.1.5.dll`.
- Mod log `Highfleet_2026-11-10_19-26-36.log`.
- User reused `/tmp/opencode/highfleet-v014-relink.perf.data` and `.txt` for the
  **v0.1.5** capture; the filename does not identify the build. The previous
  v0.1.4 long recording is now `.perf.data.old`.
- Fresh recording119.852s,89514 samples, no lost samples; main thread898751
  has83141 samples. Two uncontrolled recordings are not a performance A/B.
- User reports mild relief after enemy ships take off and disappearance of drops
  after all enemies die. No takeoff timestamp was supplied.

## Current accelerator health

Latest media snapshot:322952 hits,81634 probed misses,315 native hit-verification
samples, zero disagreements/epoch changes/evictions/collisions/contention or
lifetime disablement. The three sites total404586 calls. This confirms useful
positive lookup reuse in this session, not a particular wall-time saving.

Link removal:15970525 calls and5314329 search hops (~0.333/call), with15560938
indexed removals and zero mapping mismatches. Contacts remain pooled and direct
parts appends report no anomalies. The old native bookkeeping targets have not
returned in this profile.

## Enemy death is a clue, not exclusive AI attribution

The log explicitly records `result1 (enemies gone)` at19:32:00. The nearby
snapshot changes from410 bodies at19:31:59 to95 at19:32:05, then remains95
at19:32:11. Another battle starts at19:32:12. Earlier enemies-gone summaries
occur at19:28:25 and19:30:15.

The newest120s capture ends at roughly19:32:28, so its late workload drop is
consistent with the19:32:00 transition, but the header's wall-clock completion
time is not an exact perf/log clock synchronization. Do not invent exact enemy
death sample boundaries or label every low-sample interval as the same event.

Removing enemies also removes targets, shots, effects, aircraft/ground contact
work and many active bodies. AI can be the direct cost, an indirect workload
amplifier, or neither. Self samples in an AI body cannot bound its callee cost.
Conversely, a death-correlated slowdown does not establish that AI execution
itself is responsible. Grounded versus airborne enemies also change contact and
takeoff-control workloads, without changing the enemy count.

## Fresh sampled execution shares

| Disjoint body / host symbol | Samples/share |
|---|---:|
| Effect update0x140120470 |3666 /4.095% |
| Effect rendering traversal0x14011f690 |3063 /3.422% |
| Effect animation0x140127090 |2698 /3.014% |
| Effect wrapper0x140125000 |1088 /1.215% |
| Native media search0x140178950 |231 /0.258% |
| Native relink search0x140280080 |401 /0.448% |
| Original link-removal range |51 /0.057% |
| hf_native mapped image range |3635 /4.061% |
| Wine get_vprot_range_size |4.72% |
| Wine get_basic_memory_info |2.41% |
| libc post-syscall location+0x9f818 |13.07% |

These are execution shares, not exclusive feature costs, frame-time shares or
promised savings. The native DLL share includes working accelerators and probe
code, not an observer-only overhead measurement. Rendering remains excluded.
No allocator or AI callee cost is ruled out from cold caller leaves.

## New caller-DLL memory-query evidence

All three pinned IAT observers report active=true:

| Import owner | VirtualQuery calls | Share of covered calls |
|---|---:|---:|
| hf_ai1.1.0 |703941 |1.05% |
| hf_ally1.0.4 |61081273 |90.98% |
| hf_mc0.1.0 |5348069 |7.97% |

These counts do not measure duration and exclude QoL, native/Wine internals and
dynamically resolved APIs. The final10s interval alone contains22153 AI,
3726467 ally and106559 multicore queries. The imported guard itself is unchanged.
Source/binary caller attribution is required before assigning ally's aggregate
to panels, debug overlays, precombat logic or another feature. No panel/render
hook or global region cache is authorized as an inference from this count.

## Measurement requirements for the next update

`hf_perf0.6.0` still fails its whole initialization because its AI CALL at
0x14001123a is owned by installed hf_ai1.1.0's ally tick-skip. Existing skip
behavior is an inherited confound; it is not to be extended or silently removed.
The next AI measurement needs a **disjoint** verified native boundary and must
include native callees, preserve cadence/arguments/return/unwind behavior, and
report exactly which ship/controller paths are covered. Blind detour chaining
and bypassing byte guards are not valid solutions.

An optional caller-address observer at the known IAT boundary can separate
memory-query callers without patching a panel or altering a guard. It should
capture the original Win64 return address before a Rust prologue, use a fixed
sampling/probe budget, and call the same API with the same arguments/results.
Raw RVAs must not be called named features until the installed binary/source
mapping is verified. No new region caching is justified by this experiment.

## hf_perf initialization repair — v0.6.1

The user correctly prioritized restoring the existing profiler rather than
duplicating it in hf_native. The profiler's tracked sources were selectively
imported from benchmark HEAD aa9c5dd onto main, without historical captures or
unrelated dirty documentation. No benchmark-worktree changes were overwritten.

Optional AI span validation now warns and skips the entire three-site group
when any native CALL differs. Validation completes before any group patch is
prepared; the conflicting tick-skip instruction is never overwritten/chained.
Core frame/task profiling can install normally. Unexpected preparation errors
and core installation failures still abort rather than hide partial failures.

Core `ai_us` reads the game's inclusive AI-task timer, including its callees,
subject to the existing fresh-sample guards. This provides the first useful
AI-versus-physics/game/video split without optional CALL-site spans. The separate
driver/scan columns remain blank, not zero. Task timings are wall durations, not
exclusive CPU samples or presented FPS; absent/stale native timer samples stay
blank too. No new AI hooks or caller-attribution probes ship in this repair.

The old live config enabled dormant engine/HUD/render optimizations which never
activated because initialization failed. Deployment sets only `enabled:false`
and explicitly `profiling:true`, preserving the remaining settings/output path
and backing up the original config. Thus repairing profiling does not activate
excluded optimizations. Existing hf_ai cadence, native guards and other DLLs
remain unchanged. Formatting, strict Windows release Clippy and cross-build pass;
runtime initialization and CSV validity require the user's next launch.
