# Native engine sidesteps through injected DLLs — HighFleet 1.163

Date: 2026-11-10. Source baseline: `highfleet-allies` **main**, `3030289`.
Research only: no source behavior changes, builds, deployment, game launch or
attachment, native edits or Ghidra writes. DeepSeek V4.1 Flash 1017 reviews were
explicitly authorized by the user; parent synthesis and verification follow.

## Evidence boundary

Current measured baseline: [Nov 10 heavy-battle perf analysis](../../highfleet-benchmark/docs/perf-heavy-battle-2026-11-10.md).
Native reverse-link removal accounts for 12.14% of sampled user CPU. Wine memory
query routines account for 9.78%, incompletely attributed. Neither figure is
a frame-time share, exclusive mod cost or promised saving. Native bookkeeping
is a confirmed hotspot; the proposition that all mod code is cost-free is not.

The goal is to remove repeated native work while retaining allies, native combat
cadence and supported object/allocator contracts. A DLL can replace a bounded
native subsystem; it does not need to parallelize or rewrite the entire engine.

## Main versus perf branches

| Branch/worktree | Reviewed head | Relevant state |
|---|---|---|
| `main`, `highfleet-allies/` | `3030289` | hf_ally 1.0.4; hf_ai 1.0.0; no profiling/tick-skip module in AI |
| `perf/ai-profile-v1`, `/tmp/opencode/frame-profile-2026-11-08/mods` | `3fc8a05` | hf_ai 1.1.0, optional profiling and default-on ally tick skip |
| `perf/highfleet-benchmark`, `highfleet-benchmark/` | `ff2c91c` | hf_perf 0.6.0, native AI spans and experimental engine optimizations |

Read-only branch comparison, no checkout/cherry-pick/merge. The AI perf branch
shares an older ancestor with main and lacks subsequent ally refactors; do not
replace main wholesale with that worktree.

### Captured hook conflict and behavioral confound

- `perf/ai-profile-v1:ai/src/tick_skip.rs:19-29,40-44,80-116,137-169`
  wraps native `CALL aiShipDriver` at **0x14001123a**. Positive-side kind-1
  ships skip the driver on alternate game-clock parity. It skips decisions AND
  per-tick timer advances, so this is a gameplay/cadence trade-off.
- `perf/highfleet-benchmark:perf/src/ai_spans.rs:35-40, validate/prepare`
  claims the **same instruction** and requires original bytes.
- `perf/src/lib.rs:175-184` prepares profiling and then AI spans with error
  propagation before permanent patch installation. Thus this conflict is not
  merely an empty optional span; initialization fails.
- The Nov 10 launch log line 128 confirms tick-skip prepared; line 169 reports
  hf_perf's mismatch at 0x14001123a. Live Ghidra disassembly of SomethingAITask
  confirms that native site calls 0x1400739c0 and resumes at 0x14001123f.

Therefore the captured low direct native AI samples do not describe main's
full-cadence AI. The log/source support competing hook owners, rather than a
proven 1.163 executable mismatch. The link-search samples remain valid, but
this distinction prevents generalizing the whole capture to unmodified cadence.

### Integration decision

Keep native-engine acceleration behavior-neutral and independently configurable
on main. Do not inherit default-on AI tick skipping as its implementation.
Assign one owner to each native patch site; measurements should observe that
owner's wrapper or use disjoint sites, not bypass guards or chain unknown jumps.
This is an integration requirement, not a proposal for a generic hook framework.

The reviewed perf branches do **not** yet implement native reverse-registration
indexing or a retained ship-topology/dynamic-stats split. They contain useful
measurement infrastructure and other engine experiments, but their ally tick
skip is a different product choice. Existing callback-local panel discovery
sharing is already documented in `docs/ally-performance.md`; it should not be
presented as a new native-engine architectural solution.

## Validation rules for any native replacement

1. **Compare decisions without performing side effects twice.** For destructive
   link operations, a shadow implementation may predict which cell native code
   will remove; it must not unlink/free it as well. Re-running CalculateStats
   is not a harmless oracle if it changes histories, budgets, links or commands.
2. **Record the exact executable and loaded DLL identities.** Main, perf branch,
   source versions and installed binaries are distinct facts. A changed source
   branch does not remove a hook from an already-running process.
3. **Keep one operation owner and a known disabled path.** Startup-only byte
   guards, no blind jump chaining, no live unload. Once native state depends on
   a replacement, a mid-battle disable requires a proven resynchronization path;
   otherwise disable before activation or at a safe scene boundary.
4. **Measure wall-time outcomes separately from sampled attribution.** Use the
   same battle/save, deployment, view and configuration with repeated paired
   runs. Compare frame p50/p95 and simulation-to-wall-time ratio where reliable
   timers exist. Sample shares do not prove wall-time gains or reproducible RNG.
5. **Budget the accelerator itself.** Index lookups, lifecycle tracking, dirty
   tracking, snapshot copies and validation can replace one cost with another.
   Report hit rates, fallback counts, update costs and memory high-water marks.
6. **Do not promise to solve the whole slowdown from this hotspot.** Removing
   12.14% of sampled user CPU would still leave most sampled work. It supports
   the first vertical slice, not a forecast that heavy battles will reach 50 Hz.

## First native replacement: an indexed registration plane

The engine's reverse-reference list serves two distinct responsibilities:
membership/lifetime notification, and locating a registration to remove. The
DLL need not replace ownership or suppress reference notifications to replace
the expensive lookup. Keep the native list authoritative; retain a private
`(Link identity, Node lifetime) → registration cell` index alongside it.

What disappears is **removal-position dependence**: unregistering a reference
no longer means searching all older references. Native list order, count,
allocation ownership, Link id/node updates and teardown notifications remain.
This is a bounded representation adapter, not a replacement engine/ECS.

### Stronger caller attribution from the original capture

Parent re-parsed all 428 RemoveNodeLink samples using **both** live prologues,
not a scan for code-looking stack words. RemoveNodeLink allocates 0x28 bytes;
SetLink pushes RDI and allocates 0x20. Its caller return address is therefore
at sampled `[RSP+0x58]`, while `[RSP+0x28]` returns to SetLink:

- 428/428 immediate returns are 0x140194e06 in SetLink.
- **426/428 outer returns are 0x140049c88 in Node_UpdateDesowning.**
- Its call at 0x140049c83 passes `RCX = part+0xb0`, `RDX = 0`: the owner-clear
  operation, not targeting/ballistics. The other two samples return elsewhere.

This supersedes the earlier **401** code-word candidates and their incomplete
unwind caveat for this one additional caller level. It attributes sampled time,
not call counts: 426 slow samples do not mean only 426 owner clears occurred.
The five target Node pointers' actual object types remain unobserved.

### Additional lifecycle sites independently verified by the parent

Besides AddNodeLink/RemoveNodeLink/Link_UnlinkFromNode already verified for the
capture, live decompilation confirms inline scan/unlink/free logic in
SLink_Constructor (0x140194c20) and SLink_ScalarDeletingDestructor (0x140194e50).
The constructor initializes its embedded Link to null before the branch; its
presence in a byte search does not establish an active workload or a complete
mutation audit. Hooking only the two sampled removers is not a lifecycle proof.

**Important teardown correction:** live Node_Destructor instructions are:

```
0x14027f3a0  MOV RAX,[RBX+0x50]       ; reference-list sentinel
0x14027f3a4  MOV RCX,[RAX+0x8]        ; sentinel.prev, LAST cell
0x14027f3aa  MOV RCX,[RCX+0x10]       ; its Link*
0x14027f3ae  CALL SetLink             ; clears it via head-first RemoveNodeLink
```

AddNodeLink appends at sentinel.prev. Thus this loop selects the **tail**, not
the head as initially reported by the link-agent. With n entries and ordinary
successful removals its searches can total `n + (n-1) + … + 1`: quadratic
search work. This is a static mechanism, not measured destruction time in the
battle capture. Do not reorder teardown notifications on the assumption that
their order is unobservable. An exact index preserves the order naturally.

### Feasibility gates

- Establish all mutators, including inline unlink/free, vector relocation or
  reassignment, Node destruction and reconstruction, exceptional cleanup, and
  any third-party hooks. Byte-pattern searches are evidence, not exhaustive proof.
- **Never dereference a possibly freed cached cell to validate its liveness.**
  Backlink/data checks detect inconsistencies only after lifetime is established.
  Scene epochs alone do not prevent same-scene destruction and address reuse.
- A missed insertion can be a safe miss/native lookup. A missed removal can
  leave a dangling entry: lookup validation alone does not repair that contract.
- Native-allocated cells remain native-allocated. Reuse/migration/free-list
  designs change the allocation protocol and are separate later experiments,
  not required for the first lookup accelerator.
- No full replacement of the reverse list itself until its readers and mutation
  semantics are positively understood. "No other reader found" is insufficient.

First measure search-hop distributions by caller, including teardown. A tail
probe is attractive for confirmed tail-removal paths; a two-ended scan may help
back-half matches but adds work for near-front hits. Neither should ship merely
because it has no persistent state. Index only lists whose observed search
cost justifies maintenance; any thresholds need evidence, not arbitrary constants.

## Complementary boundary: phase-local contact storage

The boundary-agent proposes replacing individual contact-record heap lifetimes
with a private storage region, while the native engine still builds/consumes
the same records. Parent independently verified these live 1.163 sites:

| Operation | Site | Verified behavior |
|---|---|---|
| Allocate payload | 0x140284590 | New(0x48) in b2CListener_PostSolve |
| Register references | 0x1402845d2 / 0x1402845de | SetLink on the record's two native Links |
| Allocate list cell | 0x1402846ac | New(0x18) |
| Destroy references | 0x140286631 / 0x140286639 | native Link destructors in PhysicTask cleanup |
| Free payload / cell | 0x140286646 / 0x140286681 | native frees after consumption |

This boundary removes per-record allocations, not the reference searches or
physics computations. Runtime allocator cost is unmeasured; it is not the first
replacement merely because the allocation sizes are known. First establish all
normal/exceptional destruction paths and whether any consumer retains records.

### Parent corrections to the arena proposal

- Use an ownership/range registry belonging to the accelerator before classifying
  a pointer. Do not inspect arbitrary `ptr - header_size` for a magic signature.
  Non-owned pointers must use the original deallocator; original-allocation
  fallbacks are expected, not anomalies.
- Disabling new arena allocations must **not** disable dispatch for outstanding
  arena-owned frees. Drain live allocations before resetting/reusing storage.
  Wrong-thread allocation can fall back, but a later arena-owned free still
  needs correct ownership handling. A single unconditional "disabled → native"
  branch is not a valid allocator kill switch.
- An extra frame of retention is not a lifetime proof. Two alternating arenas
  do not automatically detect stale references; poisoning memory does not
  automatically detect reads. Prove no escape before any bulk reset.
- Uninitialized bytes and native Link addresses cannot be compared by raw-record
  hash across allocations/runs. Compare initialized logical payload/order and
  observable events; run the native destructors exactly once.

These corrections are required design gates, not implemented code. Reject the
arena if the measured boundary is small or its lifecycle cannot be closed.

## Second boundary: stop rendering from rematerializing the simulation

The stats-agent found that native `DrawBattleShipPanel` invokes full
CalculateStats even after the PreTask pass. Parent independently verified
the panel's zero arguments and call at **0x1402fe109**, then the following
reads of ship bounds at +0x1840/+0x1844/+0x1850.

But CalculateStats is **not a pure getter**. Parent verified its history call
at **0x140080d4e**, timer stores to +0xc70 at **0x140080db7**, and thrust-sum
zero stores at **0x140080dec / 0x140080df3 / 0x140080dfa**. The agent documents
additional part power/activation and derived-list writes. A native render call
can therefore advance simulation-related state, not merely read it.

### Architectural target

Use separate representations rather than one multifunctional native routine:

```
native simulation mutations (same main thread and order)
        │
        ├── authoritative native state + required reference notifications
        │
        └── one bounded projection at a proven observation point
                    │
                    └── read-only panel records → draw/composite
```

A read model is not a second authoritative ship. It contains only the initialized
scalars/marker records needed for rendering, copied under a proven lifetime;
it never invents owning native Link/CVec copies or writes back stale snapshots.
Snapshot cost must be less than the work removed. Existing mod-side discovery
sharing already exists; the proposed new boundary is native simulation work
triggered from presentation.

There are **two different product contracts**:

1. **Equivalent acceleration:** replace repeated calculations only after proving
   all required outputs and side effects are identical for that call. A same-frame
   stamp, input fingerprint or replay of a few timer/history stores is not that
   proof. PreTask and Video have physics/AI/Game mutations between them. A small
   guarded ally-panel fast path is research, not yet a safe drop-in solution.
2. **Intentional simulation/presentation separation:** make panel rendering
   read-only and ensure simulation history/timers are advanced by simulation
   alone. This removes the entire hidden simulation-on-draw contract, but may
   change existing gameplay behavior. It requires an explicit behavior decision,
   not a performance flag silently described as equivalent.

The agent's suggestion that this explains an old thrust-history plateau remains
an untraced hypothesis. The ResetStatAccumulators samples include every caller;
they do not measure the panel's exclusive share.

### Longer-term target: retained topology, dynamic reduction

Compile stable ship membership/order once, then update dynamic scalar results
without destroying and rebuilding every category list. Conceptually:

```
attach/detach/equipment transitions → update ordered topology/membership
fuel/power/ammo/heat/control inputs → reduce dynamic values each simulation step
presentation                     → read published projection
```

The split is not simply "damage means dirty". Coverage/activation, fuel,
extinguisher charge, power routing, weapon groups/ammo selection, transforms,
control/ownership, destruction and direct stats-block writes can affect lists
or totals. Some memberships depend on outputs of the same calculation. Before
retaining native lists, positively identify mutation coverage and evaluation
order; otherwise invalidation maintenance becomes another incomplete engine.
This is the larger architectural sidestep, **not** the first implementation.

### Parent correction to sensor-allocation claims

Reset calls helper **0x1400619b0** ten times with count 0x168 and a zero value.
Live decompilation shows vector assign/fill of **360 four-byte elements**, not
unconditional freeing of ten 360-byte arrays. With sufficient size/capacity it
retains storage and fills it; allocation/free occurs on the growth path.
Thus no design here counts ten steady-state heap round trips per reset.

## Recommended route on main

| Stage | Deliverable | Operation removed | Blocking gate |
|---|---|---|---|
| 0 | One-owner, counter/shadow-only native registration observer | None yet | Correct ABI/site ownership; bounded instrumentation overhead |
| 1 | Per-caller stateless fast path, only where measured; or lifecycle-proven exact registration-cell index | Position-dependent reverse-list searches | List identity/order, all removal/free paths and reuse; no stale cache dereference |
| 2 | One panel projection experiment | Presentation-triggered full materialization | Choose equivalent side-effect contract versus explicit behavior correction |
| 3 | Retained topology + dynamic reductions for one category | Repeated discovery, temporary Link churn, membership rebuilds | Complete category input/writer/lifetime audit; identical ordered outputs |
| Conditional | Phase-local contact storage | Contact payload/cell allocator round trips | Measured allocator cost; all escape/free paths; drain-safe fallback |
| Later only | Pure-data worker for an actual measured consumer | Main-thread pure computation | Immutable copied inputs, no worker native calls, ordered checked commit, net saving |

**Preferred architecture:** a DLL-owned accelerator behind native-compatible
entry points, with native state authoritative and deliberately narrow retained
metadata. First remove the search; then remove the reason for repeated
registration/rebuild where a complete contract exists. This removes work
without lowering AI tick frequency, ship counts, collision fidelity or ally
functionality. It is a staged subsystem replacement, not a universal cache.

Do not merge the AI perf branch wholesale, replace the process allocator,
unregister supposedly temporary references before proving lifetime, infer
thread-safety from readable memory, or turn every opaque native call into an
off-thread task. Do not promise a whole-battle frame rate from one hotspot.

## Review provenance and remaining gaps

Three authorized `openrouter/deepseek/deepseek-v4.1-flash-1017` agents produced
disjoint scratch reports under `/tmp/opencode/native-dll-architecture-2026-11-10/`:
`links.md`, `stats.md`, `boundaries.md`. Parent independently checked decisive
native instructions, source/log hook conflicts and original perf stack records.
Raw reports are working evidence, not unchecked authorities: teardown direction,
sensor-vector allocation, arena disable/ownership, and cache liveness required
correction. Neither a negative mutator search nor a checksum of native object
memory is an equivalence proof.

Still unmeasured: search-position distributions, actual five Node types/list
lengths, accelerator maintenance cost, panel-exclusive work, allocator-boundary
cost, and full-cadence main's profile in the same scenario. No in-game validation
or regression tests were performed or added; there are no behavior changes to test
yet. The first implementation proposal should be scoped to one native boundary,
with separately authorized regression coverage and controlled in-game A/B.

## Implementation record (`hf_native v0.1.0`, 2026-11-10)

Built and integrated in [`native/`](../native/README.md) on `main`:

1. **`links` (`native/src/links.rs`):** stateless whole-body replacements for
   `RemoveNodeLink` (`0x140194fb0`), `Link_UnlinkFromNode` (`0x140194d20`), and
   `SLink_ScalarDeletingDestructor` (`0x140194e50`) with newest-cell tail probe
   (`length >= 2`) and atomic hop/tail-hit/fallback counters. Persistent
   `Link -> cell` caching remains unimplemented (no positive erase-completeness
   proof).
2. **`topology` (`native/src/topology.rs`):** direct in-place
   `Link_ConstructFromBody` into `CVec<Link>` inside `Body_CollectSubtreeLinks`
   (`0x14001d02c` CALL relay + `0x14001d031` decision trampoline) when capacity
   is available, removing one temporary `New(0x18)`, `AddNodeLink`, and
   `Link_UnlinkFromNode` per collected part while keeping stock growth/copy/dtor
   at their original EH sites. Cross-frame retained `CalculateStats` lists remain
   unimplemented (self-referential dynamic gates and incomplete writer inventory).
3. **`contacts` (`native/src/contacts.rs`):** bounded DLL-private free lists
   recycling native-allocated `0x48` contact records and `0x18` list cells
   between `b2CListener_PostSolve` (`0x140284590`, `0x1402846ac`) and
   `SomethingPhysicTask` (`0x140286646`, `0x140286681`), using `C-unwind`
   near-CALL relays (`native/src/call_hook.rs`).
4. **Excluded as requested:** ship module/status panel rendering (`0x1402fe109`
   and `ally_panels`) is untouched.

## First live telemetry (`hf_native v0.1.0`, 2026-11-10 14:46–14:49)

One real heavy battle (3 friendly / 2 hostile heavies, `max_allies 2`), 9 patches
installed, no `hf_native` error and no crash. Counters are cumulative and
independent of rendered FPS.

| Module | Reading | Interpretation |
|---|---|---|
| `parts` | 11.09 M `direct`, 336 `growth`, 0 `fallback`, 0 `anomalies` | Direct in-place append served 99.997% of collected parts; the temporary `Link` register/copy/unlink/free churn is gone on that path |
| `contacts` | `pooled=0`, `hits=0`, `cached=0`, all `native=` | **Pool was inert.** Every event was rejected as "foreign thread" |
| `links` `RemoveNodeLink` | 6.15 M calls, 2.80 B hops, 56.8 k tail hits | **453.6 hops/call, 1.7% tail hits** |
| `links` `Link_UnlinkFromNode` | 30.6 M calls, 148.8 M hops, 18.6 M tail hits | 4.9 hops/call, **58.5% tail hits** |
| `links` `SLink` dtor | 0 | Never exercised in battle |

Steady-state rates over a 10 s window: `RemoveNodeLink` 51.5 k calls/s and
**23.3 M dependent pointer-chasing hops/s**; `Link_UnlinkFromNode` 278.9 k
calls/s and 1.36 M hops/s; `parts` 100 k direct appends/s.

Findings acted on in v0.1.1:

1. The thread-identity gate on the contact pool was wrong: Modloader
   initialization and the contact allocator/drain do not share a thread. The
   pool now uses only its non-blocking lock, which already existed for safety.
2. The newest-cell probe is **not** a win for `RemoveNodeLink`. Its dominant
   caller clears long-lived owner links far from the tail, so the probe adds
   loads and still scans ~454 cells. It is now a separate switch
   (`tail_probe_remove`, default off) while `Link_UnlinkFromNode` keeps it.

Still the largest measured native cost, and still blocked: the ~454-hop scan per
owner clear. Splitting the two removal workloads shows that a position-independent
handle is the only fix that targets it; a two-ended or tail scan cannot, because
the cost is the dependent-load chain, not the comparison count. That remains
gated on a positive proof that every free of a reachable registration cell passes
through a site this DLL owns.

## Resolving the measured 454-hop owner clear (`hf_native v0.1.2`)

Live telemetry from the first two seconds of the v0.1.1 run:

| Reading | Value |
|---|---|
| `RemoveNodeLink` calls | 4.41 M |
| `RemoveNodeLink` hops | 2.09 B (**474 hops/call**) |
| `RemoveNodeLink` tail hits | 0 (probe off — correctly) |
| `Link_UnlinkFromNode` | 22.5 M calls, 4.9 hops/call, 60% tail hits |
| `AddNodeLink` | not yet instrumented |

The search is not a loop-optimisation problem. Its cost is the dependent-load
chain to the cell's position, and the dominant caller clears a long-lived owner
link that sits far from either end, so neither a tail probe nor a two-ended scan
can help. The fix is to stop searching.

### Design: exact handles, complete by construction

`Node::registered_links` has exactly one insertion site (`AddNodeLink`) and
three live removal sites (`RemoveNodeLink`, `Link_UnlinkFromNode`, and the
`SLink` deleting destructor; the `SLink_Constructor` sequence is dead because it
nulls the target first). v0.1.2 replaces all four, so this DLL is the **only**
code that creates or frees an 0x18-byte registry cell.

That makes a `Link* -> (Node*, cell*)` table complete by construction: the
mapping is created in the same function that allocates and links the cell, and
destroyed by the same function that unlinks and frees it. No cross-frame object
identity, no retained game pointer, no deferred free, and no change to the
native list, its order, its count, the cell allocator or teardown notifications.

```
remove(node, link):
    if let Some(cell) = table.take(link, node)   # Link* and Node* must both match
        if cell->data == link                    # cheap re-check
            unlink(cell); free(cell); return true
        table.forget(link); downgrade counters
    return native_head_first_scan(node, link)    # always available
```

Failure handling is layered: a `Node*` mismatch is a miss, a `data` mismatch
drops the entry and falls back to the scan, a full table stops inserting (later
lookups miss), and more than 1024 mismatches permanently revert the accelerator
to native scans for the session. A missed insertion is therefore a performance
regression, never a wrong unlink.

Residual risk, stated plainly: a registry cell freed by a path this DLL does not
own would leave a stale mapping whose `data` re-check reads freed storage. The
`data` comparison cannot unlink the wrong cell, but it is the one place the
design depends on the mutation audit being complete. The audit found exactly four
`DEC [node+0x58]` sites — three replaced here, one unreachable — and no other
free of a registry cell.

Verification cost: `AddNodeLink`'s full 0x77-byte body is byte-guarded before any
patch is prepared, alongside the three removal bodies and the `New` prologue.

Expected outcome, to be confirmed by the next log rather than assumed: `hit`
approaching `remove` call count and `search_hops` collapsing from ~2 B toward
zero, with `miss`, `mismatch` and `full` staying near zero and `Link_UnlinkFromNode`
keeping its existing 60 % tail-hit rate for the cases the table misses.

### Regression and correction (v0.1.2 → v0.1.3)

v0.1.2's first live run was unplayable. Its own diagnostics named the cause:
`ins=65536` (table exactly full), `hit=0`, `full=1,562,431`.

The table used open addressing with **unbounded** probing. It filled on the
first frame, after which there was no empty slot to terminate a probe and every
insert and every lookup walked all 65 536 slots. At ~1.5 M inserts plus ~1.4 M
lookups in 90 s that is on the order of 10^11 slot visits.

Two independent design faults, both fixed in v0.1.3:

1. **Unbounded probe depth.** The table is now four-way set-associative with at
   most four probes per operation. A full or thrashing table can only lose hit
   rate; it cannot change the cost of an operation.
2. **Indexing traffic that does not need help.** Part-local lists hold a handful
   of registrations and cost a few hops to scan. Inserting them filled the table
   with entries whose lookups could never pay for the insert. Only nodes with at
   least 64 registrations are indexed now.

Eviction inside a set discards a handle, so the affected removal falls back to
the native scan; that keeps the accelerator correct while bounding its memory
and probe depth. The lesson recorded for the next iteration: an accelerator whose
worst case is unbounded is not an accelerator, and the hit-rate counters must be
read from the first run before the build is called working.

## Epoch experiments (`hf_native v0.1.4`, 2026-11-10)

The post-fix profile leaves two different architectural problems: flat steady
state and short scene-load bursts. v0.1.4 deliberately changes only the latter's
**measurement plane**. It does not claim a new acceleration, and it leaves every
v0.1.3 fast path unchanged. Twelve near-CALL relays preserve native return
addresses and call the original functions for every authoritative result.

### Distinguish a materialized index from a query-result cache

Sampled registers in the old capture contain 250 distinct `RecursiveFindByID`
IDs in 269 samples (235 singleton IDs). These are samples, not exact query
counts, but they reject the simple theory that repeated queries dominate. A
cache of previous answers would mostly miss. A build-once tree index could still
help first-time queries by replacing repeated O(subtree) searches with one tree
walk plus bounded lookups.

Live Ghidra review then rejected a map spanning the whole rebuild:

- `AssignRandomId` (`0x140280840`) rewrites `Node.id` and registered `Link.id`;
  `Scene_RebuildFromProfile` invokes it eight times and also recreates objects.
- `Escadra::Relink` alternates queries between the escadra root and the global
  scene node at `[0x147fc6f88]`; a single-current-root index would thrash.
- The global subtree size is unknown, and some virtual relink/`Body::Load` paths
  do not yet have a complete `children`/`m_id` mutation audit.

Thus the ID index remains **unimplemented**, not hidden behind a default-off
stub. It needs multi-root sizing, complete invalidation (including the
interleaved `0x140294712` ID rewrite), and the remaining mutation proof.

### Relink observer: five relays, native recursion untouched

`RecursiveFindByID` (`0x140280080`, 122 bytes) compares `Node.id` at `+0x78`,
then traverses the circular child list at `+0x100/+0x108` in pre-order. The first
duplicate ID wins; a miss returns null. The observer byte-guards this complete
body but does not replace it or its internal recursive CALL.

Two relays bracket the only verified callers of `Scene_RebuildFromProfile`
(`0x1402942a1` and `0x1402ba141`). Three more observe external queries from
`Link::Relink` (`0x140195063`) and `Joint_Relink` (`0x1401940f5`,
`0x14019414e`). Each query runs an iterative shadow DFS and then invokes the
untouched native function. The native pointer is always returned, and completed
predictions are compared with it.

The first query for each root in a pass performs a full subtree census; later
queries stop at the first match. This separates potential one-time index build
cost (`census nodes`) from repeated search cost (`visits`). Root switching,
outside-pass queries, pass brackets/timing, per-caller visits, caps and
mismatches are logged. Root addresses are cleared at pass end and never read by
the diagnostic thread.

Hard bounds are 32 roots/pass, 1024 DFS frames and 1 M visits/observation. A cap
makes only the observation incomplete; it never alters native behavior. Static
scratch storage avoids allocation and per-query initialization of a large stack.

Stage-2 admission requires zero mismatches/incomplete observations, an explained
root population, no unexpected outside-pass traffic, and measured repeated work
comfortably above full-index construction cost (initial decision rule: at least
4×). The virtual mutation audit and ID-rewrite invalidation remain mandatory even
if the arithmetic is attractive.

### Media observer: seven relays, native descriptor authoritative

`CopyNameIntoMedia` (`0x140178950`, 74 bytes) indexes 13 resource-type buckets,
not name-length buckets. It walks a singly linked descriptor list (`next +0x98`)
and performs a case-sensitive early-exit byte comparison against `name +8`.
Animation is the large shipped bucket (2249 resource blocks). The earlier
same-length description and the identification of `0x1401702b0` as a generic
effect getter were wrong; that wrapper is the sound getter (type 3).

The probe covers all seven lookup CALL sites: animation, type 11, sound, font,
texture, and two inline map-decoration animation lookups. It repeats the scan
with a 4096-candidate cap, records calls/hops/compared bytes/hit/miss/max depth
per caller, and then calls the untouched native lookup. A completed prediction
must equal the native descriptor. A bounded 2^16-entry atomic table of 64-bit
`(type,name)` hashes (16 probes, no pointers or strings) estimates repeat rate.

No media cache ships in this build. Static analysis found descriptor lifetimes
unusually long, but an insert-time index would require every parser/runtime
writer and a session/thread lifecycle contract. Measure first:

- high hops/call with low distinct/calls supports a repeat-cache investigation;
- high hops/call with mostly first-touch names points toward an insert-time
  index, not a query-result cache;
- short chains or a work budget far below the sampled CPU falsifies the theory
  that this 74-byte function explains the burst.

Both probes intentionally repeat native work during loading. **Their counters
are evidence; their wall time is not an optimization A/B.** Pass timing verifies
bracket behavior only. A later accelerator requires a separate build and a new
controlled capture. The remaining Wine/CRT/D3D host cost is not attributed or
modified by v0.1.4.
