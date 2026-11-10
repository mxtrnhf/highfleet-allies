# Changelog

## 0.1.5 — 2026-11-10

- `media_lookup_index` adds a positive-only animation-name cache at the three
  type-8 lookup CALL sites. Native first-match/duplicate selection is preserved;
  unknown names and unsupported inputs remain native. Four probes, 8192 slots,
  128-byte names, non-blocking metadata lock; no allocation or owning game copies.
  Only the published singleton is admitted. Head/manager changes flush entries;
  the guarded animation deleting-destructor vtable slot permanently disables
  acceleration before calling the untouched native destructor. One in 1024 hits
  is checked natively, and the first disagreement disables the cache.
- The old `media_lookup_probe` remains available but now defaults off and
  overrides indexing when explicitly enabled. This removes its exhaustive
  second search from the normal tester build. Negative caching is not shipped.
- Relink observation now shadows only 1/256 queries per site, with 8192-node and
  1024-frame caps, replacing the exhaustive per-query DFS and biased 32-root
  census. Root-shape/vtable histograms include outside-rebuild queries; native
  hit/miss remains authoritative. Scratch storage is synchronized by a
  non-blocking Acquire/Release guard. No ID index is shipped: a valid mutation
  and object-lifetime scope has not been established.
- Fix the four-way link table's deletion-hole lookup. An empty earlier way no
  longer hides a live later-way handle. The previous early return could fall
  back to a native scan, free the cell, and retain its stale table entry. Every
  lookup still has the same hard four-way probe limit.
- Add `host_query_probe`: counts unchanged `VirtualQuery` calls through the
  pinned, already-loaded AI 1.1.0, ally 1.0.4 and multicore 0.1.0 import entries.
  Installation validates module headers/import identity/original API pointer and
  atomically publishes each observer. Missing/different modules or existing API
  hooks are refused individually. No guards, query results or memory-region
  lifetime rules change; QoL and Wine internal calls are explicitly uncovered.
- Effect-update bookkeeping was audited further in Ghidra. No particle pool,
  update shortcut, render/animation/GPU hook, or fidelity/cadence reduction is
  shipped. Allocation benefit remains unmeasured; cold allocator-wrapper IPs do
  not bound CRT/Wine callee cost. No additional observer was added to this path.

## 0.1.4 — 2026-11-10

- Adds two **observer-only experiments** for the load-time bursts left by the
  post-v0.1.3 profile. Existing link, topology and contact optimizations are
  unchanged; no panel/render hook, AI cadence change or physics reduction.
- `scene_relink_probe`: brackets both verified callers of
  `Scene_RebuildFromProfile` and observes all three external
  `RecursiveFindByID` CALL sites. A bounded iterative shadow DFS records query
  visits, per-root full-subtree census cost, root switching, pass timing and
  native/shadow agreement. The native recursive call and native return pointer
  remain untouched.
- `media_lookup_probe`: observes all seven `CopyNameIntoMedia` CALL sites with
  bounded shadow scans, per-caller hop/byte counts, hit/miss totals and a bounded
  distinct-name hash estimate. The untouched native lookup always supplies the
  returned descriptor.
- Both probes are intentionally measurement overhead, not accelerators. Their
  load timing must not be presented as an FPS or load-time improvement.
- The proposed relink ID index is **not shipped**: IDs are re-randomized during
  rebuild, roots alternate between escadra/global trees, and some virtual
  relink/load mutation paths remain unaudited. A media pointer cache is also not
  shipped before repeat/chain measurements justify its larger lifecycle audit.
- Caps: 32 relink roots/pass, 1024 DFS frames, 1 M shadow visits/query, 4096 media
  candidates/lookup, 4096 hashed name bytes, and 16 name-hash probes. Incomplete
  observations fall back to the untouched native call and are counted.

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
