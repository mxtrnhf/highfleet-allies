# Changelog

## 0.1.3 — 2026-11-10

- **Fixes the unplayable lag introduced in 0.1.2.** The handle table used open
  addressing with unbounded probing. It filled on the first frame (`ins=65536`,
  `hit=0`), after which *every* insert and lookup walked all 65 536 slots; the
  live log showed `full=1.56 M`, i.e. ~10^11 slot visits.
- The table is now four-way set-associative with a hard cap of four probes per
  operation, so a full or thrashing table can only lose hit rate, never latency.
  Eviction is deterministic within a set and only discards a handle, which makes
  that removal fall back to the native scan.
- Only nodes with at least 64 registrations are indexed. Short lists cost a few
  hops to scan natively, so indexing them wasted the table on traffic that never
  needed help.
- `full` is renamed `evict` in the diagnostics line.

## 0.1.2 — 2026-11-10

- **Exact registration handles (`link_index`).** `AddNodeLink` is now replaced as
  well, so this DLL owns the only site that creates a 0x18-byte registry cell.
  A `Link* -> (Node*, cell*)` table is therefore created at cell creation and
  removed by the same operation that unlinks and frees the cell. Removal stops
  being a function of list position; the measured 453.6-hop owner-clear path
  becomes a table lookup. The native list, its order, its count, the native cell
  allocator and every teardown notification are unchanged.
- A handle is trusted only when both the `Link*` and `Node*` match; the cell's
  `data` field is re-checked before unlinking, and any mismatch or miss falls
  back to the original head-first scan. Table-full degrades to the scan too.
- `AddNodeLink` reproduces the native body byte-for-byte in behaviour, including
  the tail append, the wrapping count increment and the `_Xlength_error` path.
- New diagnostics: `add`, `ins`, `hit`, `miss`, `mismatch`, `full`, and per-site
  `indexed` counts.
- No panel/render hooks, AI cadence changes, or physics fidelity reductions.

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
