//! Exact registration handles for the engine's reverse-link lists.
//!
//! `Node::registered_links` is an intrusive circular list of 0x18-byte cells
//! whose payload is a `Link*`. Native removal finds a cell by walking the list
//! from the sentinel, so its cost is the position of that cell. The 2026-11-10
//! live run measured 453.6 dependent-load hops per `RemoveNodeLink` call at
//! ~51 000 calls/s, because the dominant caller (`Node_UpdateDesowning`) clears
//! and the stats pass re-adds the same owner link every frame.
//!
//! This module removes the search from the data structure instead of making the
//! loop faster. It owns **every** site that creates or frees a registry cell:
//!
//! * `AddNodeLink` (`0x140194ef0`) — the single insertion site;
//! * `RemoveNodeLink` (`0x140194fb0`);
//! * the inline unlink in `Link_UnlinkFromNode` (`0x140194d20`);
//! * the inline unlink in `SLink_ScalarDeletingDestructor` (`0x140194e50`).
//!
//! Because a cell can only come into existence through our `AddNodeLink`, a
//! `Link -> cell` mapping is created exactly when the cell is, and because all
//! three removal paths are ours, that mapping is deleted by the same operation
//! that unlinks and frees the cell. The native list, its order, its count, its
//! allocator and every teardown notification are unchanged: only the lookup is
//! replaced.
//!
//! Defence in depth, in order of severity:
//!
//! 1. A lookup requires both the `Link*` key and the `Node*` to match, so a
//!    stale or moved registration is a miss rather than a wrong unlink.
//! 2. A hit re-reads `cell->data` and compares it with the requested `Link`. If
//!    it does not match, the entry is dropped and the call falls back to the
//!    native head-first scan, which is always correct.
//! 3. Every replacement is byte-guarded against the pinned 1.163 bodies before a
//!    patch is prepared, and the whole batch is dropped on any mismatch.
//!
//! Residual risk, stated rather than hidden: if some unpatched path frees a
//! registry cell, the mapping would keep a pointer to freed storage and check 2
//! would read it. That read cannot unlink a wrong cell (the `data` comparison
//! rejects it) but it is the one place this module depends on the mutation audit
//! being complete. The audit found exactly four `DEC [node+0x58]` sites — three
//! patched here and one unreachable — and no other free of a registry cell.
//!
//! # Threading
//!
//! Every replaced routine is game-thread code and the table is unsynchronised by
//! design; the parent installs and calls these only from that thread. A foreign
//! caller would already race the native list itself.

use std::{
    cell::UnsafeCell,
    mem::transmute,
    slice,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use patchy::{PatchSession, Trampoline};

use crate::layout::{
    CELL_BYTES, CELL_LINK, CELL_NEXT, CELL_PREVIOUS, LINK_TARGET, LINK_VTABLE, LIST_COUNT,
    LIST_HEAD, NODE_LINKS,
};

// --- pinned Steam 1.163 addresses ------------------------------------------------------------

/// `AddNodeLink(Node*, Link*)`; appends a new cell at the sentinel tail.
const ADD_NODE_LINK_ADDRESS: usize = 0x1_4019_4ef0;
/// `RemoveNodeLink(Node*, Link*) -> bool`; head-first scan, unlink, `count--`, free.
const REMOVE_NODE_LINK_ADDRESS: usize = 0x1_4019_4fb0;
/// `Link_UnlinkFromNode(Link*)`; writes the Link vtable, then the same unlink body.
const LINK_UNLINK_FROM_NODE_ADDRESS: usize = 0x1_4019_4d20;
/// `SLink_ScalarDeletingDestructor(SLink*, unsigned int)`; inline unlink at `+0x48`.
const SLINK_SCALAR_DELETING_DESTRUCTOR_ADDRESS: usize = 0x1_4019_4e50;

/// Scalar `operator new(size_t)` used for the 0x18-byte registry cell.
const NATIVE_NEW_ADDRESS: usize = 0x1_4033_4094;
/// Game allocation wrapper used by the native removal sites with `RDX = 0x18`.
const NATIVE_FREE_ADDRESS: usize = 0x1_4033_40d0;
/// `std::_Xlength_error(const char*)`, the native "list too long" failure.
const LENGTH_ERROR_ADDRESS: usize = 0x1_4033_3fe7;
const LENGTH_ERROR_MESSAGE: usize = 0x1_4035_24b8;

/// Link vtable written by `Link_UnlinkFromNode` and by the SLink destructor's
/// embedded link. Pinned by the RIP-relative displacement in the native bodies.
const LINK_VTABLE_ADDRESS: usize = 0x1_4035_2310;
/// SLink vtable written at the start of `SLink_ScalarDeletingDestructor`.
const SLINK_VTABLE_ADDRESS: usize = 0x1_4036_8e18;
/// `SLink` deleting-destructor size passed to the native free wrapper.
const SLINK_BYTES: usize = 0x68;
/// Offset of the `Link` embedded in `SLink`.
const SLINK_EMBEDDED_LINK: usize = 0x48;

/// Native `Node.registered_links` sentinel pointer (`Node` offset + header offset).
const NODE_LIST_HEAD: usize = NODE_LINKS + LIST_HEAD;
/// Native `Node.registered_links.length` field (`Node` offset + length offset).
const NODE_LIST_COUNT: usize = NODE_LINKS + LIST_COUNT;
/// Stale-handle budget before the accelerator permanently reverts to native scans.
const MAX_MISMATCHES: u64 = 1024;
/// Sentinel value the native insertion path treats as "list too long".
const LIST_TOO_LONG: u64 = 0x00AA_AAAA_AAAA_AAAA;

// --- pinned native bodies --------------------------------------------------------------------

const ADD_NODE_LINK_BODY: [u8; 0x77] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48,
    0x8b, 0xf2, 0x48, 0x8d, 0x59, 0x50, 0x48, 0x8b, 0x3b, 0x48, 0xb8, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa,
    0xaa, 0xaa, 0x0a, 0x48, 0x39, 0x43, 0x08, 0x74, 0x42, 0x48, 0x89, 0x5c, 0x24, 0x20, 0x48, 0xc7,
    0x44, 0x24, 0x28, 0x00, 0x00, 0x00, 0x00, 0xb9, 0x18, 0x00, 0x00, 0x00, 0xe8, 0x63, 0xf1, 0x19,
    0x00, 0x48, 0x89, 0x70, 0x10, 0x48, 0xff, 0x43, 0x08, 0x48, 0x8b, 0x4f, 0x08, 0x48, 0x89, 0x38,
    0x48, 0x89, 0x48, 0x08, 0x48, 0x89, 0x47, 0x08, 0x48, 0x89, 0x01, 0x48, 0x8b, 0x5c, 0x24, 0x40,
    0x48, 0x8b, 0x74, 0x24, 0x48, 0x48, 0x83, 0xc4, 0x30, 0x5f, 0xc3, 0x48, 0x8d, 0x0d, 0x56, 0xd5,
    0x1b, 0x00, 0xe8, 0x80, 0xf0, 0x19, 0x00,
];

/// Scalar `operator new` prologue: `push rbx; sub rsp,0x20; mov rbx,rcx; jmp ...`.
const NATIVE_NEW_PROLOGUE: [u8; 11] = [
    0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0xeb, 0x0f,
];

const REMOVE_NODE_LINK_BODY: [u8; 0x52] = [
    0x48, 0x83, 0xec, 0x28, 0x48, 0x8b, 0x41, 0x50, 0x4c, 0x8b, 0xc1, 0x48, 0x8b, 0x08, 0x48, 0x3b,
    0xc8, 0x74, 0x0e, 0x48, 0x39, 0x51, 0x10, 0x74, 0x0f, 0x48, 0x8b, 0x09, 0x48, 0x3b, 0xc8, 0x75,
    0xf2, 0x32, 0xc0, 0x48, 0x83, 0xc4, 0x28, 0xc3, 0x48, 0x8b, 0x51, 0x08, 0x48, 0x8b, 0x01, 0x48,
    0x89, 0x02, 0x48, 0x8b, 0x11, 0x48, 0x8b, 0x41, 0x08, 0x48, 0x89, 0x42, 0x08, 0xba, 0x18, 0x00,
    0x00, 0x00, 0x49, 0xff, 0x48, 0x58, 0xe8, 0xd5, 0xf0, 0x19, 0x00, 0xb0, 0x01, 0x48, 0x83, 0xc4,
    0x28, 0xc3,
];

const LINK_UNLINK_FROM_NODE_BODY: [u8; 0x55] = [
    0x4c, 0x8b, 0x41, 0x08, 0x48, 0x8d, 0x05, 0xe5, 0xd5, 0x1b, 0x00, 0x48, 0x89, 0x01, 0x48, 0x8b,
    0xd1, 0x4d, 0x85, 0xc0, 0x74, 0x3e, 0x49, 0x8b, 0x40, 0x50, 0x48, 0x8b, 0x08, 0x48, 0x3b, 0xc8,
    0x74, 0x32, 0x48, 0x39, 0x51, 0x10, 0x74, 0x09, 0x48, 0x8b, 0x09, 0x48, 0x3b, 0xc8, 0x75, 0xf2,
    0xc3, 0x48, 0x8b, 0x51, 0x08, 0x48, 0x8b, 0x01, 0x48, 0x89, 0x02, 0x48, 0x8b, 0x11, 0x48, 0x8b,
    0x41, 0x08, 0x48, 0x89, 0x42, 0x08, 0xba, 0x18, 0x00, 0x00, 0x00, 0x49, 0xff, 0x48, 0x58, 0xe9,
    0x5c, 0xf3, 0x19, 0x00, 0xc3,
];

const SLINK_SCALAR_DELETING_DESTRUCTOR_BODY: [u8; 0x9c] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8d, 0x05, 0xb7, 0x3f, 0x1d,
    0x00, 0x8b, 0xfa, 0x48, 0x89, 0x01, 0x4c, 0x8d, 0x41, 0x48, 0x4d, 0x8b, 0x48, 0x08, 0x48, 0x8d,
    0x05, 0x9b, 0xd4, 0x1b, 0x00, 0x49, 0x89, 0x00, 0x48, 0x8b, 0xd9, 0x4d, 0x85, 0xc9, 0x74, 0x43,
    0x49, 0x8b, 0x41, 0x50, 0x48, 0x8b, 0x08, 0x48, 0x3b, 0xc8, 0x74, 0x37, 0x0f, 0x1f, 0x40, 0x00,
    0x4c, 0x39, 0x41, 0x10, 0x74, 0x0a, 0x48, 0x8b, 0x09, 0x48, 0x3b, 0xc8, 0x75, 0xf2, 0xeb, 0x23,
    0x48, 0x8b, 0x51, 0x08, 0x48, 0x8b, 0x01, 0x48, 0x89, 0x02, 0x48, 0x8b, 0x11, 0x48, 0x8b, 0x41,
    0x08, 0x48, 0x89, 0x42, 0x08, 0xba, 0x18, 0x00, 0x00, 0x00, 0x49, 0xff, 0x49, 0x58, 0xe8, 0x0d,
    0xf2, 0x19, 0x00, 0x48, 0x8b, 0xcb, 0xe8, 0x05, 0xa9, 0xff, 0xff, 0x40, 0xf6, 0xc7, 0x01, 0x74,
    0x0d, 0xba, 0x68, 0x00, 0x00, 0x00, 0x48, 0x8b, 0xcb, 0xe8, 0xf2, 0xf1, 0x19, 0x00, 0x48, 0x8b,
    0xc3, 0x48, 0x8b, 0x5c, 0x24, 0x30, 0x48, 0x83, 0xc4, 0x20, 0x5f, 0xc3,
];

// --- exact handle table ----------------------------------------------------------------------

/// Set-associative handle table with a hard bounded probe depth.
///
/// The first live build used open addressing with unbounded probing; the table
/// filled on the first frame and *every* lookup then walked all 65 536 slots,
/// which made the game unplayable. Every operation here is now at most
/// [`WAYS`] probes regardless of table state, so a full or thrashing table can
/// only cost hit rate, never latency.
///
/// Only nodes with at least [`INDEX_MIN_NODE_LEN`] registrations are inserted.
/// Short lists cost a handful of hops to scan natively, so indexing them would
/// waste the table on the traffic that does not need help.
const SET_BITS: usize = 16;
const SETS: usize = 1 << SET_BITS;
const WAYS: usize = 4;
const SLOTS: usize = SETS * WAYS;

/// Inserting only long lists keeps the live set small (a few 464-part ships),
/// so the measured load factor stays far from the set count.
const INDEX_MIN_NODE_LEN: u64 = 64;

struct HandleTable {
    keys: UnsafeCell<[usize; SLOTS]>,
    nodes: UnsafeCell<[usize; SLOTS]>,
    cells: UnsafeCell<[usize; SLOTS]>,
    hits: AtomicU64,
    misses: AtomicU64,
    mismatches: AtomicU64,
    inserted: AtomicU64,
    evictions: AtomicU64,
}

// SAFETY: every array access happens on the native game thread, which is the
// only thread that mutates registry lists. The diagnostic thread reads only the
// atomics below, never the arrays.
unsafe impl Sync for HandleTable {}

static TABLE: HandleTable = HandleTable {
    keys: UnsafeCell::new([0; SLOTS]),
    nodes: UnsafeCell::new([0; SLOTS]),
    cells: UnsafeCell::new([0; SLOTS]),
    hits: AtomicU64::new(0),
    misses: AtomicU64::new(0),
    mismatches: AtomicU64::new(0),
    inserted: AtomicU64::new(0),
    evictions: AtomicU64::new(0),
};

static INDEX_ENABLED: AtomicBool = AtomicBool::new(false);

#[inline]
fn hash(link: usize) -> u64 {
    // Fibonacci hashing; heap addresses differ mostly in the low, aligned bits.
    (link as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

#[inline]
fn set_base(link: usize) -> usize {
    ((hash(link) >> (64 - SET_BITS)) as usize) * WAYS
}

/// Records the cell created for `link` on `node`. At most [`WAYS`] probes.
///
/// # Safety
/// Game-thread only. `cell` must be a freshly created, linked registry cell.
unsafe fn index_insert(link: usize, node: usize, cell: usize) {
    let keys = unsafe { &mut *TABLE.keys.get() };
    let nodes = unsafe { &mut *TABLE.nodes.get() };
    let cells = unsafe { &mut *TABLE.cells.get() };
    let base = set_base(link);
    let mut empty = None;

    for way in 0..WAYS {
        let slot = base + way;
        let key = keys[slot];
        if key == link {
            nodes[slot] = node;
            cells[slot] = cell;
            return;
        }
        if key == 0 && empty.is_none() {
            empty = Some(slot);
        }
    }

    // Prefer an unused way; otherwise evict deterministically from this set.
    // Eviction only loses a handle (that removal falls back to the native
    // scan), so it can never produce a wrong unlink.
    let slot = empty.unwrap_or_else(|| {
        TABLE.evictions.fetch_add(1, Ordering::Relaxed);
        base + ((hash(link) >> SET_BITS) as usize & (WAYS - 1))
    });
    keys[slot] = link;
    nodes[slot] = node;
    cells[slot] = cell;
    TABLE.inserted.fetch_add(1, Ordering::Relaxed);
}

/// Removes and returns the mapping for `link` registered on `node`.
///
/// # Safety
/// Game-thread only. The returned cell is still linked; the caller must unlink
/// and free it exactly as the native routine would.
unsafe fn index_take(link: usize, node: usize) -> Option<usize> {
    let keys = unsafe { &mut *TABLE.keys.get() };
    let nodes = unsafe { &*TABLE.nodes.get() };
    let cells = unsafe { &*TABLE.cells.get() };
    let base = set_base(link);

    for way in 0..WAYS {
        let slot = base + way;
        let key = keys[slot];
        if key == 0 {
            return None;
        }
        if key != link {
            continue;
        }
        if nodes[slot] != node {
            // A registration that moved: leave the entry for its current node.
            return None;
        }
        let cell = cells[slot];
        keys[slot] = 0;
        return Some(cell);
    }
    None
}

/// Drops a mapping without freeing anything.
#[inline]
unsafe fn index_forget(link: usize) {
    let keys = unsafe { &mut *TABLE.keys.get() };
    let base = set_base(link);
    for way in 0..WAYS {
        let slot = base + way;
        if keys[slot] == link {
            keys[slot] = 0;
            return;
        }
    }
}

// --- configuration and counters --------------------------------------------------------------

/// A singleton's newest cell is also its oldest: probing it separately cannot
/// save a scan. Two entries is the smallest list with a distinct tail candidate.
const TAIL_PROBE_MIN_LENGTH: u64 = 2;

static TAIL_PROBE: AtomicBool = AtomicBool::new(false);
/// Separate switch for `RemoveNodeLink`. The live 2026-11-10 run measured
/// 453.6 hops/call with only 1.7% newest-cell hits there, so the probe was a
/// pure loss on the dominant owner-clear path; `Link_UnlinkFromNode` measured
/// 4.9 hops/call with 58.5% hits, where it clearly pays.
static TAIL_PROBE_REMOVE: AtomicBool = AtomicBool::new(false);

#[derive(Debug)]
struct SiteCounters {
    calls: AtomicU64,
    search_hops: AtomicU64,
    tail_hits: AtomicU64,
    fallbacks: AtomicU64,
    indexed: AtomicU64,
}

impl SiteCounters {
    const fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
            search_hops: AtomicU64::new(0),
            tail_hits: AtomicU64::new(0),
            fallbacks: AtomicU64::new(0),
            indexed: AtomicU64::new(0),
        }
    }

    fn snapshot(&self) -> SiteSnapshot {
        SiteSnapshot {
            calls: self.calls.load(Ordering::Relaxed),
            search_hops: self.search_hops.load(Ordering::Relaxed),
            tail_hits: self.tail_hits.load(Ordering::Relaxed),
            fallbacks: self.fallbacks.load(Ordering::Relaxed),
            indexed: self.indexed.load(Ordering::Relaxed),
        }
    }
}

static REMOVE_NODE_LINK_COUNTERS: SiteCounters = SiteCounters::new();
static LINK_UNLINK_FROM_NODE_COUNTERS: SiteCounters = SiteCounters::new();
static SLINK_DESTRUCTOR_COUNTERS: SiteCounters = SiteCounters::new();
static ADD_NODE_LINK_CALLS: AtomicU64 = AtomicU64::new(0);

/// Aggregate counters for one replaced removal site.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SiteSnapshot {
    /// Entries into the replaced function, including defensive early returns.
    pub calls: u64,
    /// Registry cells whose payload was compared with the requested `Link`.
    pub search_hops: u64,
    /// Calls where the newest cell matched and no head scan was needed.
    pub tail_hits: u64,
    /// Calls where the newest cell did not match and the head scan ran.
    pub fallbacks: u64,
    /// Calls served by an exact handle with no list search at all.
    pub indexed: u64,
}

/// Aggregate counters for every site replaced by this module.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkRemovalSnapshot {
    pub add_calls: u64,
    pub handles_inserted: u64,
    pub index_hits: u64,
    pub index_misses: u64,
    pub index_mismatches: u64,
    pub table_evictions: u64,
    pub remove_node_link: SiteSnapshot,
    pub link_unlink_from_node: SiteSnapshot,
    pub slink_destructor: SiteSnapshot,
}

/// Reads the aggregate counters without touching game memory.
pub fn snapshot() -> LinkRemovalSnapshot {
    LinkRemovalSnapshot {
        add_calls: ADD_NODE_LINK_CALLS.load(Ordering::Relaxed),
        handles_inserted: TABLE.inserted.load(Ordering::Relaxed),
        index_hits: TABLE.hits.load(Ordering::Relaxed),
        index_misses: TABLE.misses.load(Ordering::Relaxed),
        index_mismatches: TABLE.mismatches.load(Ordering::Relaxed),
        table_evictions: TABLE.evictions.load(Ordering::Relaxed),
        remove_node_link: REMOVE_NODE_LINK_COUNTERS.snapshot(),
        link_unlink_from_node: LINK_UNLINK_FROM_NODE_COUNTERS.snapshot(),
        slink_destructor: SLINK_DESTRUCTOR_COUNTERS.snapshot(),
    }
}

// --- borrowed native field access ------------------------------------------------------------

/// Reads one pointer-sized scalar field from a game object.
///
/// # Safety
///
/// `base` must be a live, readable object for the duration of the call and
/// `offset` must name a pointer-sized scalar inside it. The read is unaligned
/// but the caller must still establish that the address is mapped; this helper
/// does not prove ownership or lifetime.
#[inline]
unsafe fn read_pointer(base: *mut u8, offset: usize) -> *mut u8 {
    (base.add(offset) as *const usize).read_unaligned() as *mut u8
}

/// Reads the native cell count of a `SentinelList` header.
///
/// # Safety
///
/// As [`read_pointer`]: `base + offset` must be a readable `u64` inside a live
/// `Node` for this call.
#[inline]
unsafe fn read_length(base: *mut u8, offset: usize) -> u64 {
    (base.add(offset) as *const u64).read_unaligned()
}

/// Writes one pointer-sized scalar field in a game object.
///
/// # Safety
///
/// `base` must be a live, writable object owned by the current game-thread
/// operation, `offset` must name a pointer-sized scalar, and no other native
/// access may race this store. The store is unaligned.
#[inline]
unsafe fn write_pointer(base: *mut u8, offset: usize, value: usize) {
    (base.add(offset) as *mut usize).write_unaligned(value);
}

/// Allocates one 0x18-byte registry cell through the game's own allocator.
///
/// # Safety
///
/// Game-thread only. The native `New` may throw `std::bad_alloc`; this is an
/// `extern "C-unwind"` call so that unwinding follows the native frames.
#[inline]
unsafe fn allocate_cell() -> *mut u8 {
    let allocate: unsafe extern "C-unwind" fn(usize) -> *mut u8 = transmute(NATIVE_NEW_ADDRESS);
    allocate(CELL_BYTES)
}

/// Releases a 0x18-byte registry cell through the game's own free wrapper.
///
/// # Safety
///
/// `cell` must be a cell that the caller has already unlinked from its list and
/// that no other native object references. This must run on the game thread so
/// the game allocator sees the same thread as native removal.
#[inline]
unsafe fn free_cell(cell: *mut u8) {
    let free: unsafe extern "C-unwind" fn(*mut u8, usize) = transmute(NATIVE_FREE_ADDRESS);
    free(cell, CELL_BYTES);
}

/// Unlinks `cell` from `node`'s registry and decrements the native count.
///
/// This mirrors the native sequence exactly: write `previous->next`, then
/// reload `cell->next`/`cell->previous` and write `next->previous`, then a
/// wrapping decrement of `node.length`. It does not free the cell.
///
/// # Safety
///
/// `node` must be a live `Node`, `cell` must be a live cell currently linked in
/// that node's registry, and no other native code may access the list
/// concurrently. All three native callers already establish this contract.
#[inline]
unsafe fn unlink_cell(node: *mut u8, cell: *mut u8) {
    let previous = read_pointer(cell, CELL_PREVIOUS);
    let next = read_pointer(cell, CELL_NEXT);
    write_pointer(previous, CELL_NEXT, next as usize);

    let next = read_pointer(cell, CELL_NEXT);
    let previous = read_pointer(cell, CELL_PREVIOUS);
    write_pointer(next, CELL_PREVIOUS, previous as usize);

    let count = read_length(node, NODE_LIST_COUNT);
    (node.add(NODE_LIST_COUNT) as *mut u64).write_unaligned(count.wrapping_sub(1));
}

struct SearchResult {
    /// Matching cell, or null when the native scan would have returned a miss.
    cell: *mut u8,
    /// Number of cell payload comparisons performed.
    hops: u64,
    tail_probed: bool,
    tail_hit: bool,
    indexed: bool,
}

/// Resolves the cell for `link` on `node`: exact handle first, native scan next.
///
/// # Safety
///
/// `node` must be a live `Node` and `link` the registration the native routine
/// was called with; the same list invariants the native body assumes apply.
unsafe fn resolve(node: *mut u8, link: *mut u8, tail_probe: bool) -> SearchResult {
    if INDEX_ENABLED.load(Ordering::Relaxed) {
        if let Some(cell) = index_take(link as usize, node as usize) {
            let cell = cell as *mut u8;
            // A handle is only trusted when the cell still points at this Link.
            if read_pointer(cell, CELL_LINK) == link {
                TABLE.hits.fetch_add(1, Ordering::Relaxed);
                return SearchResult {
                    cell,
                    hops: 0,
                    tail_probed: false,
                    tail_hit: false,
                    indexed: true,
                };
            }
            // Stale or foreign mapping: drop it and let the native scan decide.
            // A handful of these can happen around teardown; a sustained rate
            // means the mutation audit is incomplete, so stop trusting the
            // table for the rest of the session and keep the native scan.
            let mismatches = TABLE.mismatches.fetch_add(1, Ordering::Relaxed) + 1;
            index_forget(link as usize);
            if mismatches > MAX_MISMATCHES {
                INDEX_ENABLED.store(false, Ordering::Relaxed);
            }
        } else {
            TABLE.misses.fetch_add(1, Ordering::Relaxed);
        }
    }
    search(node, link, tail_probe)
}

/// Performs the equivalent search, optionally testing the newest cell first.
///
/// # Safety
///
/// `node` must be a live `Node` whose `registered_links` sentinel is readable,
/// and `link` must be the `Link*` the native routine was called with. The list
/// invariants are exactly the ones the native body assumes: a circular list
/// terminating at the sentinel. The function performs no writes.
unsafe fn search(node: *mut u8, link: *mut u8, tail_probe: bool) -> SearchResult {
    let head = read_pointer(node, NODE_LIST_HEAD);
    if head.is_null() {
        return SearchResult {
            cell: std::ptr::null_mut(),
            hops: 0,
            tail_probed: false,
            tail_hit: false,
            indexed: false,
        };
    }

    if tail_probe {
        let length = read_length(node, NODE_LIST_COUNT);
        if length >= TAIL_PROBE_MIN_LENGTH {
            let newest = read_pointer(head, CELL_PREVIOUS);
            if newest != head && read_pointer(newest, CELL_LINK) == link {
                return SearchResult {
                    cell: newest,
                    hops: 1,
                    tail_probed: true,
                    tail_hit: true,
                    indexed: false,
                };
            }

            let scan = scan_from_oldest(head, link);
            return SearchResult {
                // The failed tail comparison is real work too; telemetry must
                // include it when comparing enabled/disabled search paths.
                hops: scan.hops.saturating_add(1),
                tail_probed: true,
                tail_hit: false,
                ..scan
            };
        }
    }

    scan_from_oldest(head, link)
}

/// The native oldest-first scan, unchanged except for hop accounting.
///
/// # Safety
///
/// `head` must be the live sentinel of a circular registry list; `link` is the
/// payload being searched for. Performs no writes.
unsafe fn scan_from_oldest(head: *mut u8, link: *mut u8) -> SearchResult {
    let mut cell = read_pointer(head, CELL_NEXT);
    let mut hops = 0u64;

    while cell != head {
        hops += 1;
        if read_pointer(cell, CELL_LINK) == link {
            return SearchResult {
                cell,
                hops,
                tail_probed: false,
                tail_hit: false,
                indexed: false,
            };
        }
        cell = read_pointer(cell, CELL_NEXT);
    }

    SearchResult {
        cell: std::ptr::null_mut(),
        hops,
        tail_probed: false,
        tail_hit: false,
        indexed: false,
    }
}

/// Records one completed resolution without formatting or logging.
#[inline]
fn record_search(counters: &SiteCounters, search: &SearchResult) {
    counters.calls.fetch_add(1, Ordering::Relaxed);
    if search.hops != 0 {
        counters
            .search_hops
            .fetch_add(search.hops, Ordering::Relaxed);
    }
    if search.indexed {
        counters.indexed.fetch_add(1, Ordering::Relaxed);
    }
    if search.tail_probed {
        if search.tail_hit {
            counters.tail_hits.fetch_add(1, Ordering::Relaxed);
        } else {
            counters.fallbacks.fetch_add(1, Ordering::Relaxed);
        }
    }
}

// --- replacement bodies ----------------------------------------------------------------------

/// Whole-body replacement for `AddNodeLink(Node*, Link*) -> void`.
///
/// Appends the cell exactly where the native body does (sentinel tail), then
/// records the handle. This is the only site that creates a registry cell, so
/// the handle table is complete by construction.
///
/// # Safety
///
/// Entered by the installed jump with `node` in `RCX` and `link` in `RDX`. Both
/// must be live game objects, as the native body already requires.
#[inline(never)]
unsafe extern "C-unwind" fn add_node_link(node: *mut u8, link: *mut u8) {
    if node.is_null() || link.is_null() {
        return;
    }

    let head = read_pointer(node, NODE_LIST_HEAD);
    if head.is_null() {
        return;
    }

    let count_slot = node.add(NODE_LIST_COUNT);
    let count = (count_slot as *const u64).read_unaligned();
    if count == LIST_TOO_LONG {
        // Same failure the native body raises for an impossible list length.
        let length_error: unsafe extern "C-unwind" fn(*const u8) -> ! =
            transmute(LENGTH_ERROR_ADDRESS);
        length_error(LENGTH_ERROR_MESSAGE as *const u8);
    }

    let cell = allocate_cell();
    write_pointer(cell, CELL_LINK, link as usize);
    (count_slot as *mut u64).write_unaligned(count.wrapping_add(1));

    let previous = read_pointer(head, CELL_PREVIOUS);
    write_pointer(cell, CELL_NEXT, head as usize);
    write_pointer(cell, CELL_PREVIOUS, previous as usize);
    write_pointer(head, CELL_PREVIOUS, cell as usize);
    write_pointer(previous, CELL_NEXT, cell as usize);

    ADD_NODE_LINK_CALLS.fetch_add(1, Ordering::Relaxed);
    if count >= INDEX_MIN_NODE_LEN && INDEX_ENABLED.load(Ordering::Relaxed) {
        index_insert(link as usize, node as usize, cell as usize);
    }
}

/// Whole-body replacement for `RemoveNodeLink(Node*, Link*) -> bool`.
///
/// # Safety
///
/// Entered by the installed jump with `node` in `RCX` and `link` in `RDX`. It
/// returns `true` exactly when a cell was unlinked and freed.
#[inline(never)]
unsafe extern "C-unwind" fn remove_node_link(node: *mut u8, link: *mut u8) -> bool {
    if node.is_null() {
        REMOVE_NODE_LINK_COUNTERS
            .calls
            .fetch_add(1, Ordering::Relaxed);
        return false;
    }

    let tail_probe = TAIL_PROBE_REMOVE.load(Ordering::Relaxed);
    let result = resolve(node, link, tail_probe);
    record_search(&REMOVE_NODE_LINK_COUNTERS, &result);

    if result.cell.is_null() {
        return false;
    }

    unlink_cell(node, result.cell);
    free_cell(result.cell);
    true
}

/// Whole-body replacement for `Link_UnlinkFromNode(Link*)`.
///
/// The native body always writes the Link vtable before testing the target; it
/// leaves `link->target` set after a successful removal and does not free the
/// `Link` itself. Both properties are preserved.
///
/// # Safety
///
/// Entered by the installed jump with `link` in `RCX`. It must be a live `Link`
/// for the duration of the call; valid callers establish that.
#[inline(never)]
unsafe extern "C-unwind" fn link_unlink_from_node(link: *mut u8) {
    if link.is_null() {
        LINK_UNLINK_FROM_NODE_COUNTERS
            .calls
            .fetch_add(1, Ordering::Relaxed);
        return;
    }

    // Native loads the target before writing the vtable; keep that order even
    // though the fields do not overlap.
    let node = read_pointer(link, LINK_TARGET);
    write_pointer(link, LINK_VTABLE, LINK_VTABLE_ADDRESS);
    if node.is_null() {
        LINK_UNLINK_FROM_NODE_COUNTERS
            .calls
            .fetch_add(1, Ordering::Relaxed);
        return;
    }

    let tail_probe = TAIL_PROBE.load(Ordering::Relaxed);
    let result = resolve(node, link, tail_probe);
    record_search(&LINK_UNLINK_FROM_NODE_COUNTERS, &result);

    if result.cell.is_null() {
        return;
    }

    unlink_cell(node, result.cell);
    free_cell(result.cell);
}

/// Whole-body replacement for `SLink_ScalarDeletingDestructor(SLink*, unsigned int)`.
///
/// Preserves the native order: write the SLink vtable, load and test the
/// embedded Link's target, write the Link vtable, inline-unlink a matching cell,
/// call the base destructor, then free `this` with size 0x68 when deleting bit 0
/// is set. Returns `this`, as the native body does.
///
/// # Safety
///
/// Entered by the installed jump with `this` in `RCX` and the deleting flag in
/// `EDX`. `this` must be a live `SLink` whose base destructor is valid for this
/// image; valid virtual/delete callers establish that.
#[inline(never)]
unsafe extern "C-unwind" fn slink_scalar_deleting_destructor(
    this: *mut u8,
    deleting_flag: u32,
) -> *mut u8 {
    if this.is_null() {
        SLINK_DESTRUCTOR_COUNTERS
            .calls
            .fetch_add(1, Ordering::Relaxed);
        return this;
    }

    write_pointer(this, 0, SLINK_VTABLE_ADDRESS);

    let link = this.add(SLINK_EMBEDDED_LINK);
    let node = read_pointer(link, LINK_TARGET);
    write_pointer(link, LINK_VTABLE, LINK_VTABLE_ADDRESS);

    if !node.is_null() {
        let tail_probe = TAIL_PROBE.load(Ordering::Relaxed);
        let result = resolve(node, link, tail_probe);
        record_search(&SLINK_DESTRUCTOR_COUNTERS, &result);

        if !result.cell.is_null() {
            unlink_cell(node, result.cell);
            free_cell(result.cell);
        }
    } else {
        SLINK_DESTRUCTOR_COUNTERS
            .calls
            .fetch_add(1, Ordering::Relaxed);
    }

    let base_destructor: unsafe extern "C-unwind" fn(*mut u8) = transmute(SLINK_BASE_DESTRUCTOR);
    base_destructor(this);

    if deleting_flag & 1 != 0 {
        let free: unsafe extern "C-unwind" fn(*mut u8, usize) = transmute(NATIVE_FREE_ADDRESS);
        free(this, SLINK_BYTES);
    }

    this
}

/// `SLink` base-class destructor, called after the inline unlink.
const SLINK_BASE_DESTRUCTOR: usize = 0x1_4018_f7d0;

// --- preparation -----------------------------------------------------------------------------

/// Validates one pinned native body before any patch is added.
///
/// # Safety
///
/// `address` must be readable for `expected.len()` bytes in the live process.
unsafe fn validate_body(name: &str, address: usize, expected: &[u8]) -> Result<(), String> {
    let actual = slice::from_raw_parts(address as *const u8, expected.len());
    if actual == expected {
        return Ok(());
    }

    let mismatch = actual
        .iter()
        .zip(expected)
        .position(|(found, wanted)| found != wanted)
        .unwrap_or(0);
    Err(format!(
        "links: {name} at {address:#x} does not match the pinned 1.163 body; \
         first mismatch at byte {mismatch} (found {:#04x}, expected {:#04x})",
        actual[mismatch], expected[mismatch]
    ))
}

/// Prepares an absolute jump to a whole-body replacement.
///
/// The source range is the complete native function, so the trampoline never
/// replays a relocated prologue and no RIP-relative instruction is copied.
///
/// # Safety
///
/// The caller must have validated the whole source body and must keep the
/// replacement loaded for the process lifetime.
unsafe fn prepare_replacement(
    session: &mut PatchSession,
    name: &str,
    address: usize,
    size: usize,
    replacement: *const (),
) -> Result<(), String> {
    let mut trampoline = Trampoline::new();
    trampoline.absolute_jump(replacement as usize);
    session
        .detour_trampoline(address, size, trampoline)
        .map(|_| ())
        .map_err(|error| {
            format!("links: cannot prepare {name} replacement at {address:#x}: {error}")
        })
}

/// Validates the four pinned native bodies and prepares whole-body replacement
/// detours in `session`.
///
/// `tail_probe` enables the newest-cell test on lists of at least
/// [`TAIL_PROBE_MIN_LENGTH`] entries for `Link_UnlinkFromNode` and the `SLink`
/// destructor. `tail_probe_remove` enables it separately for `RemoveNodeLink`,
/// whose dominant caller clears long-lived owner links far from the tail.
///
/// No patch is added unless every native body matches.
///
/// # Safety
///
/// The process image must be the supported Steam 1.163 executable at its
/// preferred base. The caller must install and keep this DLL resident for the
/// process lifetime, and no other module may own the same four entry points.
/// All four routines must remain game-thread-only, as native code assumes.
pub unsafe fn prepare(
    session: &mut PatchSession,
    tail_probe: bool,
    tail_probe_remove: bool,
    exact_index: bool,
) -> Result<(), String> {
    validate_body("AddNodeLink", ADD_NODE_LINK_ADDRESS, &ADD_NODE_LINK_BODY)?;
    validate_body(
        "RemoveNodeLink",
        REMOVE_NODE_LINK_ADDRESS,
        &REMOVE_NODE_LINK_BODY,
    )?;
    validate_body(
        "Link_UnlinkFromNode",
        LINK_UNLINK_FROM_NODE_ADDRESS,
        &LINK_UNLINK_FROM_NODE_BODY,
    )?;
    validate_body(
        "SLink_ScalarDeletingDestructor",
        SLINK_SCALAR_DELETING_DESTRUCTOR_ADDRESS,
        &SLINK_SCALAR_DELETING_DESTRUCTOR_BODY,
    )?;
    validate_body("New", NATIVE_NEW_ADDRESS, &NATIVE_NEW_PROLOGUE)?;

    prepare_replacement(
        session,
        "AddNodeLink",
        ADD_NODE_LINK_ADDRESS,
        ADD_NODE_LINK_BODY.len(),
        add_node_link as *const (),
    )?;
    prepare_replacement(
        session,
        "RemoveNodeLink",
        REMOVE_NODE_LINK_ADDRESS,
        REMOVE_NODE_LINK_BODY.len(),
        remove_node_link as *const (),
    )?;
    prepare_replacement(
        session,
        "Link_UnlinkFromNode",
        LINK_UNLINK_FROM_NODE_ADDRESS,
        LINK_UNLINK_FROM_NODE_BODY.len(),
        link_unlink_from_node as *const (),
    )?;
    prepare_replacement(
        session,
        "SLink_ScalarDeletingDestructor",
        SLINK_SCALAR_DELETING_DESTRUCTOR_ADDRESS,
        SLINK_SCALAR_DELETING_DESTRUCTOR_BODY.len(),
        slink_scalar_deleting_destructor as *const (),
    )?;

    TAIL_PROBE.store(tail_probe, Ordering::Relaxed);
    TAIL_PROBE_REMOVE.store(tail_probe && tail_probe_remove, Ordering::Relaxed);
    // Enabled last: permanent installation follows this call, so no insertion
    // hook can run before the table is ready. A dropped session leaves the patch
    // set empty and the flag irrelevant.
    INDEX_ENABLED.store(exact_index, Ordering::Relaxed);
    Ok(())
}
