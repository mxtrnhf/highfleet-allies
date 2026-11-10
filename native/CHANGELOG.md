# Changelog

## 0.1.1 — 2026-11-10

- **Contact pool was entirely inert in the first live run.** Every allocate/free
  reported the calling thread as foreign to the Modloader init thread, so all
  events fell back to native `New`/`free`. The pool now admits any thread and
  relies solely on the non-blocking lock (contention already fell back). The
  `foreign_thread` counter is replaced by `inactive` (hooks that ran before the
  pool had a capacity).
- **Per-site tail probe.** The live run measured `RemoveNodeLink` at 453.6
  hops/call with 1.7% newest-cell hits and `Link_UnlinkFromNode` at 4.9
  hops/call with 58.5% hits. `tail_probe_remove` (new, default `false`) gates the
  probe on `RemoveNodeLink`; `tail_probe` still enables it for
  `Link_UnlinkFromNode` and the `SLink` destructor.
- No change to hooks, patched bytes, semantics, or to any panel/render path.

## 0.1.0 — 2026-11-10

- Initial experimental native-engine DLL for Steam 1.163 (`hf_native_v0.1.0.dll`).
- `links`: whole-body replacements for `RemoveNodeLink` (`0x140194fb0`), `Link_UnlinkFromNode` (`0x140194d20`), and `SLink_ScalarDeletingDestructor` (`0x140194e50`) with newest-cell tail probing (`length >= 2`) and hop/hit counters.
- `topology`: direct in-place `Link_ConstructFromBody` append in `Body_CollectSubtreeLinks` (`0x14001d02c` / `0x14001d031`), eliminating temporary `Link` registration/copy/unlink/free churn on the capacity-available path while preserving native growth and exception cleanup sites.
- `contacts`: scoped native-allocated free-list reuse for `b2CListener_PostSolve` (`0x48` record at `0x140284590`, `0x18` cell at `0x1402846ac`) and `SomethingPhysicTask` drain (`0x140286646`, `0x140286681`) via `C-unwind` near-CALL relays.
- Explicitly excluded / blocked: no panel or read-model hooks; no AI tick skipping or cadence changes; no persistent raw-cell `Link` cache or cross-frame retained `CalculateStats` category vectors (blocked on positive erase-completeness and writer-inventory proofs).
