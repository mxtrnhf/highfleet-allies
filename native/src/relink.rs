//! Sampled observer for `RecursiveFindByID` relink queries.
//!
//! v0.1.4 measured 258 621 queries, 217 393 of them (84 %) outside the two
//! `Scene_RebuildFromProfile` brackets, with a saturated 32-entry root table. The
//! old shape assumed this walk is load-scoped, i.e. bracketing the rebuild
//! function would cover the traffic. Live review falsifies that assumption.
//!
//! The three external query sites live inside `Link::Relink` (`0x140195010`) and
//! `Joint_Relink` (`0x140194080`), which are reached only through virtual relink
//! slots: `Link` vtable slot 0 and `Joint` vtable slot `+0x40`. Every relink
//! entry derives its search root as `target ? target : this` and propagates that
//! one root to children, joints and bodies:
//!
//! * `Node::Relink` `0x140280c70` (`TEST RDX,RDX` / `CMOVNZ RBP,RDX`, then
//!   `CALL [this+0x80]`, `[child vtable+0x80]`, `[joint vtable+0x40]`);
//! * `Body::relink` `0x14004b880` (same root derivation, then `Node::Relink`);
//! * `GUI::Relink` `0x140165f70` (root, then `Body::relink` plus three GUI links);
//! * `Node_RelinkAndNotify` `0x140104950` and `Node_SetRelinkTarget`
//!   `0x1401049c0` (root, then `Node::Relink`);
//! * the `+0x80` Escadra relink slot (`0x1401048a0`, Ghidra
//!   `Node_ReparentChildren`) is the only static multi-root case: it relinks
//!   member link `+0x48` against the global scene node `[0x147fc6f88]` and
//!   member links `+0x78`/`+0x60` against the escadra.
//!
//! Those entry points are virtual targets referenced by many vtables, so the
//! hosting operation is not recoverable from a rebuild bracket: engine code
//! reaches the same slots during load, reparenting, spawning, cloning and GUI
//! relink. `Scene_RebuildFromProfile` `0x1402943f0` is a rebuild boundary, not a
//! scope that contains the walk traffic: its two call sites are one call inside
//! `Load` `0x140293f90` and one call inside the huge `SomethingWithLevelSerias`
//! `0x1402ac970` (immediately after `Get_Profile` `0x140292a90`), so neither
//! bracket can bound the relinks that the surrounding load/spawn code performs.
//!
//! This module therefore observes rather than indexes:
//!
//! * every query records its relay site, whether a rebuild bracket is active,
//!   the native result class (hit or miss), a bounded bucket of the root's
//!   direct child count and the root's first object word (its vtable pointer) in
//!   a bounded first-seen identity histogram;
//! * every 256th query per site additionally runs a bounded shadow pre-order
//!   walk, compares its prediction with the untouched native result, and inserts
//!   the root pointer into a bounded sampled identity table.
//!
//! No ID index is installed, and neither is an "operation-local" one. `m_id` is
//! rewritten by `AssignRandomId` `0x140280840` from load, spawn, clone,
//! part-edit, rescue, editor and shop paths (dozens of static callers), while
//! clone and reparent operations also mutate child lists (`Creature_CloneSubtree`
//! `0x1401bfa10` calls `AssignRandomId` and `add`). A mapping built for one walk
//! therefore has no proven lifetime across the next one, and the measured root
//! set already exceeded the old 32-entry table, so there is no proven bounded
//! multi-root closure either. The untouched native query remains the sole
//! authority for every returned Node.
//!
//! Cost is bounded and small. The per-query path is a few atomic increments plus
//! reads inside the object native already dereferences; it never walks the tree.
//! The shadow walk runs on 1/256 of queries with an 8 192-node and 1 024-frame
//! cap, and a try-lock makes a concurrent sample skip instead of sharing scratch
//! storage. Native object concurrency is not assumed anywhere: a sample that
//! cannot claim the scratch space is counted as contended and skipped.
//!
//! Diagnostic snapshots read atomics only. Root pointers are stored as scalar
//! fingerprints for the duration of the run; nothing is dereferenced by the
//! diagnostic thread, and no fingerprint is used to answer a query.

use std::{
    cell::UnsafeCell,
    mem::transmute,
    slice,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Instant,
};

use patchy::PatchSession;

use crate::{
    call_hook,
    layout::{CELL_LINK, CELL_NEXT, NODE_CHILDREN_COUNT, NODE_CHILDREN_HEAD, NODE_ID},
};

const RECURSIVE_FIND_BY_ID: usize = 0x1_4028_0080;
const RECURSIVE_FIND_BY_ID_BODY: [u8; 122] = [
    0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xf2, 0x48, 0x8b, 0xf9,
    0x48, 0x39, 0x51, 0x78, 0x75, 0x0e, 0x48, 0x8b, 0xc1, 0x48, 0x8b, 0x74, 0x24, 0x38, 0x48, 0x83,
    0xc4, 0x20, 0x5f, 0xc3, 0x48, 0x83, 0xb9, 0x08, 0x01, 0x00, 0x00, 0x00, 0x48, 0x89, 0x5c, 0x24,
    0x30, 0x74, 0x35, 0x48, 0x8b, 0x81, 0x00, 0x01, 0x00, 0x00, 0x48, 0x8b, 0x18, 0x48, 0x3b, 0xd8,
    0x74, 0x26, 0x48, 0x8b, 0x43, 0x10, 0x48, 0x8b, 0x1b, 0x48, 0x39, 0x70, 0x78, 0x74, 0x1b, 0x48,
    0x8b, 0xd6, 0x48, 0x8b, 0xc8, 0xe8, 0xa6, 0xff, 0xff, 0xff, 0x48, 0x85, 0xc0, 0x75, 0x0b, 0x48,
    0x3b, 0x9f, 0x00, 0x01, 0x00, 0x00, 0x75, 0xda, 0x33, 0xc0, 0x48, 0x8b, 0x5c, 0x24, 0x30, 0x48,
    0x8b, 0x74, 0x24, 0x38, 0x48, 0x83, 0xc4, 0x20, 0x5f, 0xc3,
];

const SCENE_REBUILD_FROM_PROFILE: usize = 0x1_4029_43f0;
const SCENE_REBUILD_PROLOGUE: [u8; 23] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x48, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41,
    0x56, 0x41, 0x57, 0x48, 0x8d, 0x68, 0xa1,
];

/// Deterministic 1-in-256 phase sample, per query site.
const SAMPLE_MASK: u64 = 0x00ff;

/// Shadow walk caps for a sampled query. Both make an observation incomplete;
/// they never influence the native result. The visit cap is roughly 27x the
/// v0.1.4 mean of 304 visits per query, so only a genuinely large tail is cut,
/// and `sample_incomplete` makes that visible.
const MAX_SAMPLE_VISITS: u64 = 8192;
const MAX_SHADOW_DEPTH: usize = 1024;

/// Root-shape buckets: index 0 is a root-id hit (native returns without reading
/// the child count), then the root's direct child count. Index is documented by
/// `root_shape`.
const ROOT_SHAPES: usize = 9;

/// First-seen root vtable identities per run; overflow is counted separately.
const ROOT_VTABLE_SLOTS: usize = 12;

/// Sampled root identity fingerprints. Bounded and only touched by the sampled
/// path; saturation is reported instead of evicting.
const SAMPLE_ROOT_SLOTS: usize = 4096;
const SAMPLE_ROOT_PROBES: usize = 8;
const SAMPLE_ROOT_SHIFT: u32 = 12;
const HASH_MULTIPLIER: u64 = 0x9e37_79b9_7f4a_7c15;
const _: () = assert!(
    SAMPLE_ROOT_SLOTS == 1 << SAMPLE_ROOT_SHIFT,
    "sampled root table size and hash shift disagree"
);

#[derive(Clone, Copy)]
struct CallSite {
    name: &'static str,
    address: usize,
    call: [u8; 5],
    replacement: *const (),
}

// The pointer is an immutable replacement-function address used by the relay.
unsafe impl Sync for CallSite {}

const REBUILD_LOAD: usize = 0;
const REBUILD_LEVEL: usize = 1;
const REBUILD_CALL_SITES: [CallSite; 2] = [
    CallSite {
        name: "load rebuild",
        address: 0x1_4029_42a1,
        call: [0xe8, 0x4a, 0x01, 0x00, 0x00],
        replacement: load_rebuild as *const (),
    },
    CallSite {
        name: "level rebuild",
        address: 0x1_402b_a141,
        call: [0xe8, 0xaa, 0xa2, 0xfd, 0xff],
        replacement: level_rebuild as *const (),
    },
];

const LINK_QUERY: usize = 0;
const JOINT_MASTER_QUERY: usize = 1;
const JOINT_A_QUERY: usize = 2;
const QUERY_CALL_SITES: [CallSite; 3] = [
    CallSite {
        name: "Link relink query",
        address: 0x1_4019_5063,
        call: [0xe8, 0x18, 0xb0, 0x0e, 0x00],
        replacement: link_query as *const (),
    },
    CallSite {
        name: "Joint master query",
        address: 0x1_4019_40f5,
        call: [0xe8, 0x86, 0xbf, 0x0e, 0x00],
        replacement: joint_master_query as *const (),
    },
    CallSite {
        name: "Joint A query",
        address: 0x1_4019_414e,
        call: [0xe8, 0x2d, 0xbf, 0x0e, 0x00],
        replacement: joint_a_query as *const (),
    },
];

struct SiteCounters {
    sequence: AtomicU64,
    calls: AtomicU64,
    outside_rebuild: AtomicU64,
    hits: AtomicU64,
    misses: AtomicU64,
    samples: AtomicU64,
    sample_nodes: AtomicU64,
    sample_max_nodes: AtomicU64,
    sample_complete: AtomicU64,
    sample_incomplete: AtomicU64,
    sample_mismatches: AtomicU64,
    sample_contended: AtomicU64,
}

impl SiteCounters {
    const fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            calls: AtomicU64::new(0),
            outside_rebuild: AtomicU64::new(0),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            samples: AtomicU64::new(0),
            sample_nodes: AtomicU64::new(0),
            sample_max_nodes: AtomicU64::new(0),
            sample_complete: AtomicU64::new(0),
            sample_incomplete: AtomicU64::new(0),
            sample_mismatches: AtomicU64::new(0),
            sample_contended: AtomicU64::new(0),
        }
    }

    fn snapshot(&self) -> QuerySnapshot {
        QuerySnapshot {
            calls: self.calls.load(Ordering::Relaxed),
            outside_rebuild: self.outside_rebuild.load(Ordering::Relaxed),
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            samples: self.samples.load(Ordering::Relaxed),
            sample_nodes: self.sample_nodes.load(Ordering::Relaxed),
            sample_max_nodes: self.sample_max_nodes.load(Ordering::Relaxed),
            sample_complete: self.sample_complete.load(Ordering::Relaxed),
            sample_incomplete: self.sample_incomplete.load(Ordering::Relaxed),
            sample_mismatches: self.sample_mismatches.load(Ordering::Relaxed),
            sample_contended: self.sample_contended.load(Ordering::Relaxed),
        }
    }
}

static QUERY_COUNTERS: [SiteCounters; 3] = [const { SiteCounters::new() }; 3];

static REBUILDS_STARTED: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static REBUILDS_COMPLETED: AtomicU64 = AtomicU64::new(0);
static NESTED_REBUILDS: AtomicU64 = AtomicU64::new(0);
static REBUILD_DEPTH: AtomicU64 = AtomicU64::new(0);
static REBUILD_TOTAL_US: AtomicU64 = AtomicU64::new(0);
static REBUILD_MAX_US: AtomicU64 = AtomicU64::new(0);

static ROOT_SHAPES_ALL: [AtomicU64; ROOT_SHAPES] = [const { AtomicU64::new(0) }; ROOT_SHAPES];
static ROOT_SHAPES_OUTSIDE: [AtomicU64; ROOT_SHAPES] = [const { AtomicU64::new(0) }; ROOT_SHAPES];

/// Bounded first-seen root-vtable identities. `key == 0` is an empty slot; every
/// later distinct vtable is counted in `other` instead of evicting a slot.
struct VtableHistogram {
    keys: [AtomicU64; ROOT_VTABLE_SLOTS],
    counts: [AtomicU64; ROOT_VTABLE_SLOTS],
    other: AtomicU64,
}

impl VtableHistogram {
    const fn new() -> Self {
        Self {
            keys: [const { AtomicU64::new(0) }; ROOT_VTABLE_SLOTS],
            counts: [const { AtomicU64::new(0) }; ROOT_VTABLE_SLOTS],
            other: AtomicU64::new(0),
        }
    }

    /// Counts the root object's first word. The word is a scalar fingerprint that
    /// is stored in atomics and never dereferenced again, so a recycled address
    /// can only merge two classes in the report; it cannot change a query result.
    /// The compare-exchange chain keeps slots consistent even if two threads ever
    /// reached this path.
    ///
    /// # Safety
    /// `root` must be the live Node the native call is about to search.
    unsafe fn record(&self, root: *mut u8) {
        let key = (root as *const u64).read_unaligned();
        if key == 0 {
            self.other.fetch_add(1, Ordering::Relaxed);
            return;
        }
        for slot in 0..ROOT_VTABLE_SLOTS {
            let slot_key = &self.keys[slot];
            let current = slot_key.load(Ordering::Relaxed);
            if current == key {
                self.counts[slot].fetch_add(1, Ordering::Relaxed);
                return;
            }
            if current == 0 {
                match slot_key.compare_exchange(0, key, Ordering::Relaxed, Ordering::Relaxed) {
                    Ok(_) => {
                        self.counts[slot].fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    Err(claimed) if claimed == key => {
                        self.counts[slot].fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    Err(_) => continue,
                }
            }
        }
        self.other.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> ([RootVtable; ROOT_VTABLE_SLOTS], u64) {
        let entries = std::array::from_fn(|slot| RootVtable {
            key: self.keys[slot].load(Ordering::Relaxed),
            count: self.counts[slot].load(Ordering::Relaxed),
        });
        (entries, self.other.load(Ordering::Relaxed))
    }
}

static ROOT_VTABLES_ALL: VtableHistogram = VtableHistogram::new();
static ROOT_VTABLES_OUTSIDE: VtableHistogram = VtableHistogram::new();

static SAMPLE_ROOT_KEYS: [AtomicU64; SAMPLE_ROOT_SLOTS] =
    [const { AtomicU64::new(0) }; SAMPLE_ROOT_SLOTS];
static SAMPLE_DISTINCT_ROOTS: AtomicU64 = AtomicU64::new(0);
static SAMPLE_REPEAT_ROOTS: AtomicU64 = AtomicU64::new(0);
static SAMPLE_ROOT_TABLE_FULL: AtomicU64 = AtomicU64::new(0);

/// Exclusive use of the shadow stack. A contended sample skips its walk.
static SHADOW_BUSY: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QuerySnapshot {
    pub calls: u64,
    pub outside_rebuild: u64,
    pub hits: u64,
    pub misses: u64,
    pub samples: u64,
    pub sample_nodes: u64,
    pub sample_max_nodes: u64,
    pub sample_complete: u64,
    pub sample_incomplete: u64,
    pub sample_mismatches: u64,
    pub sample_contended: u64,
}

/// One scalar root-vtable fingerprint and its query count. `key == 0` is empty.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RootVtable {
    pub key: u64,
    pub count: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RelinkSnapshot {
    pub rebuild_passes_load: u64,
    pub rebuild_passes_level: u64,
    pub rebuilds_completed: u64,
    pub nested_rebuilds: u64,
    pub rebuild_total_us: u64,
    pub rebuild_max_us: u64,
    pub queries: u64,
    pub queries_outside_rebuild: u64,
    pub hits: u64,
    pub misses: u64,
    pub samples: u64,
    pub sample_nodes: u64,
    pub sample_max_nodes: u64,
    pub sample_complete: u64,
    pub sample_incomplete: u64,
    pub sample_mismatches: u64,
    pub sample_contended: u64,
    /// Distinct roots seen by the sampled path, frequency-weighted by query
    /// count. A bounded lower bound, not a census of every root.
    pub sample_distinct_roots: u64,
    pub sample_repeat_roots: u64,
    pub sample_root_table_full: u64,
    /// Direct-child-count buckets over every query, and over the queries that
    /// ran outside an active rebuild bracket.
    pub root_shapes: [u64; ROOT_SHAPES],
    pub root_shapes_outside: [u64; ROOT_SHAPES],
    /// Root-vtable identities over every query, and over the outside queries.
    /// Scalar fingerprints only; `other` counts identities beyond the slots.
    pub root_vtables: [RootVtable; ROOT_VTABLE_SLOTS],
    pub root_vtable_other: u64,
    pub root_vtables_outside: [RootVtable; ROOT_VTABLE_SLOTS],
    pub root_vtable_other_outside: u64,
    pub sites: [QuerySnapshot; 3],
}

/// Reads atomics only; never inspects a Node, a root or the shadow scratch.
pub fn snapshot() -> RelinkSnapshot {
    let sites = std::array::from_fn(|site| QUERY_COUNTERS[site].snapshot());
    let (root_vtables, root_vtable_other) = ROOT_VTABLES_ALL.snapshot();
    let (root_vtables_outside, root_vtable_other_outside) = ROOT_VTABLES_OUTSIDE.snapshot();
    let mut result = RelinkSnapshot {
        rebuild_passes_load: REBUILDS_STARTED[REBUILD_LOAD].load(Ordering::Relaxed),
        rebuild_passes_level: REBUILDS_STARTED[REBUILD_LEVEL].load(Ordering::Relaxed),
        rebuilds_completed: REBUILDS_COMPLETED.load(Ordering::Relaxed),
        nested_rebuilds: NESTED_REBUILDS.load(Ordering::Relaxed),
        rebuild_total_us: REBUILD_TOTAL_US.load(Ordering::Relaxed),
        rebuild_max_us: REBUILD_MAX_US.load(Ordering::Relaxed),
        sample_distinct_roots: SAMPLE_DISTINCT_ROOTS.load(Ordering::Relaxed),
        sample_repeat_roots: SAMPLE_REPEAT_ROOTS.load(Ordering::Relaxed),
        sample_root_table_full: SAMPLE_ROOT_TABLE_FULL.load(Ordering::Relaxed),
        root_shapes: std::array::from_fn(|shape| ROOT_SHAPES_ALL[shape].load(Ordering::Relaxed)),
        root_shapes_outside: std::array::from_fn(|shape| {
            ROOT_SHAPES_OUTSIDE[shape].load(Ordering::Relaxed)
        }),
        root_vtables,
        root_vtable_other,
        root_vtables_outside,
        root_vtable_other_outside,
        sites,
        ..RelinkSnapshot::default()
    };

    for site in &result.sites {
        result.queries += site.calls;
        result.queries_outside_rebuild += site.outside_rebuild;
        result.hits += site.hits;
        result.misses += site.misses;
        result.samples += site.samples;
        result.sample_nodes += site.sample_nodes;
        result.sample_max_nodes = result.sample_max_nodes.max(site.sample_max_nodes);
        result.sample_complete += site.sample_complete;
        result.sample_incomplete += site.sample_incomplete;
        result.sample_mismatches += site.sample_mismatches;
        result.sample_contended += site.sample_contended;
    }
    result
}

#[derive(Clone, Copy)]
struct Frame {
    node: *mut u8,
    sentinel: *mut u8,
    next_cell: *mut u8,
    entered: bool,
}

impl Frame {
    const fn new(node: *mut u8) -> Self {
        Self {
            node,
            sentinel: std::ptr::null_mut(),
            next_cell: std::ptr::null_mut(),
            entered: false,
        }
    }
}

struct ShadowCell(UnsafeCell<[Frame; MAX_SHADOW_DEPTH]>);

// SAFETY: The scratch array is only accessed by the thread that wins
// `SHADOW_BUSY`. The walk is non-recursive and holds the flag for its whole
// duration, so there is never a second reader or writer. Diagnostics do not
// touch this storage.
unsafe impl Sync for ShadowCell {}

static SHADOW_STACK: ShadowCell = ShadowCell(UnsafeCell::new(
    [Frame::new(std::ptr::null_mut()); MAX_SHADOW_DEPTH],
));

struct Observation {
    predicted: *mut u8,
    visits: u64,
    complete: bool,
}

/// Bounded iterative equivalent of the native pre-order search.
///
/// # Safety
/// `root` must be the live native Node supplied to the query. Child-list
/// invariants are the same ones `RecursiveFindByID` requires; caps turn a
/// malformed or oversized walk into an incomplete observation, never a result.
/// The caller must hold `SHADOW_BUSY`.
unsafe fn observe_find(root: *mut u8, id: u64) -> Observation {
    // SAFETY: The caller holds the scratch try-lock. Only frames below `depth`
    // are read, and each is initialized before it is pushed.
    let stack = unsafe { &mut *SHADOW_STACK.0.get() };
    let mut depth = 1usize;
    stack[0] = Frame::new(root);
    let mut visits = 0u64;

    while depth != 0 {
        if visits >= MAX_SAMPLE_VISITS {
            return Observation {
                predicted: std::ptr::null_mut(),
                visits,
                complete: false,
            };
        }

        let frame = &mut stack[depth - 1];
        if !frame.entered {
            frame.entered = true;
            visits += 1;

            if (frame.node.add(NODE_ID) as *const u64).read_unaligned() == id {
                // Native stops at the first pre-order match, so this observation
                // does too; every sampled comparison is like for like.
                return Observation {
                    predicted: frame.node,
                    visits,
                    complete: true,
                };
            }

            let children = (frame.node.add(NODE_CHILDREN_COUNT) as *const u64).read_unaligned();
            if children == 0 {
                depth -= 1;
                continue;
            }

            let sentinel =
                (frame.node.add(NODE_CHILDREN_HEAD) as *const usize).read_unaligned() as *mut u8;
            frame.sentinel = sentinel;
            frame.next_cell = (sentinel.add(CELL_NEXT) as *const usize).read_unaligned() as *mut u8;
        }

        if frame.next_cell == frame.sentinel {
            depth -= 1;
            continue;
        }

        // Native advances the cell before testing the child and then recurses.
        // Keeping that order preserves sibling order and first-duplicate choice.
        let cell = frame.next_cell;
        let child = (cell.add(CELL_LINK) as *const usize).read_unaligned() as *mut u8;
        frame.next_cell = (cell.add(CELL_NEXT) as *const usize).read_unaligned() as *mut u8;

        if depth == MAX_SHADOW_DEPTH {
            return Observation {
                predicted: std::ptr::null_mut(),
                visits,
                complete: false,
            };
        }
        stack[depth] = Frame::new(child);
        depth += 1;
    }

    Observation {
        predicted: std::ptr::null_mut(),
        visits,
        complete: true,
    }
}

/// Buckets the root's direct child count, plus a bucket for a root-id hit.
///
/// # Safety
/// `root` must be the live Node the native call is about to search.
///
/// Native reads `root+0x78` first and returns immediately on a match; it reads
/// `root+0x108` only otherwise, and never touches the child sentinel when the
/// count is zero. Mirroring that order keeps the observer from reading a field
/// the native call would not, and bucket 0 then means the query id is the root's
/// own id, so native answers at the root without descending.
///
/// Buckets: 0 root-id hit, 1 no children, 2 one, 3 two to four, 4 five to
/// sixteen, 5 seventeen to sixty-four, 6 sixty-five to 256, 7 257 to 1024,
/// 8 more.
unsafe fn root_shape(root: *mut u8, id: u64) -> usize {
    let root_id = (root.add(NODE_ID) as *const u64).read_unaligned();
    if root_id == id {
        return 0;
    }
    let children = (root.add(NODE_CHILDREN_COUNT) as *const u64).read_unaligned();
    match children {
        0 => 1,
        1 => 2,
        2..=4 => 3,
        5..=16 => 4,
        17..=64 => 5,
        65..=256 => 6,
        257..=1024 => 7,
        _ => 8,
    }
}

/// Records one sampled root pointer in a bounded probe table.
///
/// Only the pointer value is used; `root` is non-null because the native call
/// dereferences it, and nothing is read through it here.
///
/// This is a fingerprint set, not a cache. It answers "how often does the
/// sampled path revisit a root", and is never consulted to answer a query. The
/// table saturates instead of evicting; `sample_root_table_full` counts samples
/// that could not be inserted.
fn record_sampled_root(root: *mut u8) {
    let key = (root as u64) | 1;
    let base = (key.wrapping_mul(HASH_MULTIPLIER) >> (64 - SAMPLE_ROOT_SHIFT)) as usize
        & (SAMPLE_ROOT_SLOTS - 1);

    for probe in 0..SAMPLE_ROOT_PROBES {
        let slot = &SAMPLE_ROOT_KEYS[(base + probe) & (SAMPLE_ROOT_SLOTS - 1)];
        let found = slot.load(Ordering::Relaxed);
        if found == key {
            SAMPLE_REPEAT_ROOTS.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if found == 0 {
            match slot.compare_exchange(0, key, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => {
                    SAMPLE_DISTINCT_ROOTS.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                Err(claimed) if claimed == key => {
                    SAMPLE_REPEAT_ROOTS.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                Err(_) => continue,
            }
        }
    }
    SAMPLE_ROOT_TABLE_FULL.fetch_add(1, Ordering::Relaxed);
}

/// Runs the bounded shadow walk for one sampled query.
///
/// # Safety
/// `root` must be the live native Node supplied to the query. The caller must
/// have decided to sample this query. Ordinary queries never enter here.
unsafe fn sample_query(root: *mut u8, id: u64) -> Option<Observation> {
    if SHADOW_BUSY
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return None;
    }

    // SAFETY: The scratch space is exclusively ours until the flag is stored back
    // below, and the walk cannot unwind or re-enter this relay.
    let observation = unsafe { observe_find(root, id) };
    SHADOW_BUSY.store(false, Ordering::Release);
    record_sampled_root(root);
    Some(observation)
}

fn count_sample(counters: &SiteCounters, observation: Option<Observation>, native_result: *mut u8) {
    let Some(observation) = observation else {
        counters.sample_contended.fetch_add(1, Ordering::Relaxed);
        return;
    };

    counters.samples.fetch_add(1, Ordering::Relaxed);
    counters
        .sample_nodes
        .fetch_add(observation.visits, Ordering::Relaxed);
    counters
        .sample_max_nodes
        .fetch_max(observation.visits, Ordering::Relaxed);

    if !observation.complete {
        counters.sample_incomplete.fetch_add(1, Ordering::Relaxed);
    } else {
        counters.sample_complete.fetch_add(1, Ordering::Relaxed);
        if observation.predicted != native_result {
            counters.sample_mismatches.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// # Safety
/// Forwards the native query ABI. Native remains authoritative: the shadow
/// observation can only add counters, and an incomplete or disagreeing
/// observation changes nothing about the returned Node.
unsafe fn run_query(root: *mut u8, id: u64, site: usize) -> *mut u8 {
    let counters = &QUERY_COUNTERS[site];
    let inside_rebuild = REBUILD_DEPTH.load(Ordering::Relaxed) != 0;

    // SAFETY: The relay only replaces the CALL to the native query, so `root`
    // and `id` satisfy the same contract native is about to require. The shape
    // read mirrors native's own order, and the first object word is only
    // fingerprinted, never followed.
    let shape = unsafe { root_shape(root, id) };
    // SAFETY: Same live root; only its first word is read.
    unsafe { ROOT_VTABLES_ALL.record(root) };

    counters.calls.fetch_add(1, Ordering::Relaxed);
    ROOT_SHAPES_ALL[shape].fetch_add(1, Ordering::Relaxed);
    if !inside_rebuild {
        counters.outside_rebuild.fetch_add(1, Ordering::Relaxed);
        ROOT_SHAPES_OUTSIDE[shape].fetch_add(1, Ordering::Relaxed);
        // SAFETY: Same live root; only its first word is read.
        unsafe { ROOT_VTABLES_OUTSIDE.record(root) };
    }

    let sampled = counters.sequence.fetch_add(1, Ordering::Relaxed) & SAMPLE_MASK == 0;
    let observation = if sampled {
        // SAFETY: Same live query inputs; only the sampled path walks the tree.
        unsafe { sample_query(root, id) }
    } else {
        None
    };

    let native: unsafe extern "C" fn(*mut u8, u64) -> *mut u8 = transmute(RECURSIVE_FIND_BY_ID);
    let native_result = unsafe { native(root, id) };

    if native_result.is_null() {
        counters.misses.fetch_add(1, Ordering::Relaxed);
    } else {
        counters.hits.fetch_add(1, Ordering::Relaxed);
    }
    if sampled {
        count_sample(counters, observation, native_result);
    }

    native_result
}

macro_rules! query_wrapper {
    ($function:ident, $site:expr) => {
        unsafe extern "C" fn $function(root: *mut u8, id: u64) -> *mut u8 {
            // SAFETY: The wrapper preserves the validated two-argument query ABI.
            unsafe { run_query(root, id, $site) }
        }
    };
}

query_wrapper!(link_query, LINK_QUERY);
query_wrapper!(joint_master_query, JOINT_MASTER_QUERY);
query_wrapper!(joint_a_query, JOINT_A_QUERY);

/// # Safety
/// Called by the rebuild wrapper on the game thread before entering native
/// rebuild. Only a depth counter and two atomics are touched.
unsafe fn rebuild_enter(site: usize) {
    REBUILDS_STARTED[site].fetch_add(1, Ordering::Relaxed);
    if REBUILD_DEPTH.fetch_add(1, Ordering::Relaxed) != 0 {
        NESTED_REBUILDS.fetch_add(1, Ordering::Relaxed);
    }
}

/// # Safety
/// Balances one `rebuild_enter`, including C-unwind exit.
unsafe fn rebuild_leave() {
    // Each guard balances exactly one entry, on both normal and unwind paths.
    REBUILD_DEPTH.fetch_sub(1, Ordering::Relaxed);
}

struct RebuildGuard;

impl Drop for RebuildGuard {
    fn drop(&mut self) {
        // SAFETY: The guard is created immediately after rebuild_enter and
        // dropped on the same thread on both normal and C-unwind exits.
        unsafe { rebuild_leave() };
    }
}

/// # Safety
/// Calls the native rebuild by its verified absolute address and returns its
/// full RAX result unchanged. The guard only leaves the depth bracket.
unsafe fn run_rebuild(scene: *mut u8, site: usize) -> u64 {
    unsafe { rebuild_enter(site) };
    let guard = RebuildGuard;
    let started = Instant::now();

    let native: unsafe extern "C-unwind" fn(*mut u8) -> u64 = transmute(SCENE_REBUILD_FROM_PROFILE);
    let result = unsafe { native(scene) };

    let elapsed_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    REBUILDS_COMPLETED.fetch_add(1, Ordering::Relaxed);
    REBUILD_TOTAL_US.fetch_add(elapsed_us, Ordering::Relaxed);
    REBUILD_MAX_US.fetch_max(elapsed_us, Ordering::Relaxed);
    drop(guard);
    result
}

unsafe extern "C-unwind" fn load_rebuild(scene: *mut u8) -> u64 {
    // SAFETY: Validated CALL replacement for the native rebuild signature.
    unsafe { run_rebuild(scene, REBUILD_LOAD) }
}

unsafe extern "C-unwind" fn level_rebuild(scene: *mut u8) -> u64 {
    // SAFETY: Validated CALL replacement for the native rebuild signature.
    unsafe { run_rebuild(scene, REBUILD_LEVEL) }
}

/// Validates both rebuild brackets, all three external queries, the native query
/// body and the rebuild prologue before preparing any of the five CALL relays.
/// No native query instruction is replaced and no ID index is installed.
///
/// # Safety
/// Runs once at quiescent Modloader initialization on the supported fixed-base
/// Steam 1.163 image. No other module may own these CALL instructions. The DLL
/// and relays remain resident for the process lifetime.
pub unsafe fn prepare(session: &mut PatchSession) -> Result<(), String> {
    // SAFETY: The parent establishes the supported executable mapping.
    let query_body = unsafe {
        slice::from_raw_parts(
            RECURSIVE_FIND_BY_ID as *const u8,
            RECURSIVE_FIND_BY_ID_BODY.len(),
        )
    };
    if query_body != RECURSIVE_FIND_BY_ID_BODY {
        return Err("relink: RecursiveFindByID body differs at 0x140280080".into());
    }

    // SAFETY: Same supported-image contract; only the fixed prologue is read.
    let rebuild_prologue = unsafe {
        slice::from_raw_parts(
            SCENE_REBUILD_FROM_PROFILE as *const u8,
            SCENE_REBUILD_PROLOGUE.len(),
        )
    };
    if rebuild_prologue != SCENE_REBUILD_PROLOGUE {
        return Err("relink: Scene_RebuildFromProfile prologue differs".into());
    }

    for site in REBUILD_CALL_SITES.iter().chain(QUERY_CALL_SITES.iter()) {
        // SAFETY: Five bytes cover the complete native CALL being validated.
        let actual = unsafe { slice::from_raw_parts(site.address as *const u8, 5) };
        if actual != site.call {
            return Err(format!(
                "relink: {} CALL differs at {:#x}",
                site.name, site.address
            ));
        }
    }

    for site in REBUILD_CALL_SITES.iter().chain(QUERY_CALL_SITES.iter()) {
        // SAFETY: Each replacement implements its validated native CALL ABI and
        // delegates to the original absolute target for the authoritative result.
        unsafe {
            call_hook::prepare_call(session, site.address, &site.call, site.replacement)?;
        }
    }
    Ok(())
}
