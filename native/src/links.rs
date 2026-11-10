//! Stateless accelerator for the engine's reverse-registration `Link` removals.
//!
//! `Node::registered_links` is an intrusive circular list of 0x18-byte cells
//! whose payload is a `Link*`. Three native bodies remove a cell by scanning
//! oldest-first from the sentinel:
//!
//! * `RemoveNodeLink` (`0x140194fb0`);
//! * the inline unlink in `Link_UnlinkFromNode` (`0x140194d20`);
//! * the inline unlink in `SLink_ScalarDeletingDestructor` (`0x140194e50`).
//!
//! `Node_Destructor` instead repeatedly takes the newest cell
//! (`sentinel.previous`) and clears that `Link`, so teardown walks the list
//! from the head on every iteration and is quadratic. This module replaces the
//! three removal bodies with byte-equivalent implementations that may test the
//! newest cell before falling back to the native oldest-first order.
//!
//! The accelerator keeps **no side table, no cached cell pointer and no
//! registration of its own**. Every call re-reads the native list from the live
//! `Node`, so it has no erase-coverage or cached-lifetime obligation: a missed
//! probe is just the native scan, and a removed cell is the same cell the list
//! would have yielded. That is deliberate. A persistent `Link -> cell` index
//! would need a positive proof that every free of a reachable cell passes
//! through a hooked site; the existing negative byte search does not provide
//! one, and dereferencing a cached cell to validate it is already a
//! use-after-free read (see `/tmp/opencode/native-dll-architecture-2026-11-10/
//! links.md` §3 D1 and §4 E5).
//!
//! Layout offsets come from [`crate::layout`], which derives them from the
//! pinned `highfleet-rs` revision. This module only performs borrowed scalar
//! and raw-pointer loads/stores; it never materialises an owning copy of a
//! game object and never frees anything except the 0x18-byte registry cell the
//! native routine would have freed.
//!
//! # Threading
//!
//! The replaced routines are game-thread code, and the parent must install and
//! call them only from that thread. The hot path deliberately does not call
//! `GetCurrentThreadId`: a foreign-thread call must still perform the native
//! removal rather than silently leak the registration, and a per-removal thread
//! check would only add cost to the measured path.

use std::{
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

/// `RemoveNodeLink(Node*, Link*) -> bool`; head-first scan, unlink, `count--`, free.
const REMOVE_NODE_LINK_ADDRESS: usize = 0x1_4019_4fb0;
/// `Link_UnlinkFromNode(Link*)`; writes the Link vtable, then the same unlink body.
const LINK_UNLINK_FROM_NODE_ADDRESS: usize = 0x1_4019_4d20;
/// `SLink_ScalarDeletingDestructor(SLink*, unsigned int)`; inline unlink at `+0x48`.
const SLINK_SCALAR_DELETING_DESTRUCTOR_ADDRESS: usize = 0x1_4019_4e50;

/// Game allocation wrapper used by the native removal sites with `RDX = 0x18`.
const NATIVE_FREE_ADDRESS: usize = 0x1_4033_40d0;
/// `SLink` base-class destructor, called after the inline unlink in the SLink dtor.
const SLINK_BASE_DESTRUCTOR_ADDRESS: usize = 0x1_4018_f7d0;

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

// --- pinned native bodies --------------------------------------------------------------------
//
// Whole-function bodies, inclusive of the final `RET`. `prepare` validates all
// of them before adding any patch, so the inline `CALL`/`JMP` displacements also
// pin `NATIVE_FREE_ADDRESS`, `SLINK_BASE_DESTRUCTOR_ADDRESS` and both vtable
// leaves. Any drift, including a relocated helper, fails preparation.

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

// --- configuration and counters --------------------------------------------------------------

/// A singleton's newest cell is also its oldest: probing it separately cannot
/// save a scan. Two entries is the smallest list with a distinct tail candidate.
const TAIL_PROBE_MIN_LENGTH: u64 = 2;

static TAIL_PROBE: AtomicBool = AtomicBool::new(false);

#[derive(Debug)]
struct SiteCounters {
    calls: AtomicU64,
    search_hops: AtomicU64,
    tail_hits: AtomicU64,
    fallbacks: AtomicU64,
}

impl SiteCounters {
    const fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
            search_hops: AtomicU64::new(0),
            tail_hits: AtomicU64::new(0),
            fallbacks: AtomicU64::new(0),
        }
    }

    fn snapshot(&self) -> SiteSnapshot {
        SiteSnapshot {
            calls: self.calls.load(Ordering::Relaxed),
            search_hops: self.search_hops.load(Ordering::Relaxed),
            tail_hits: self.tail_hits.load(Ordering::Relaxed),
            fallbacks: self.fallbacks.load(Ordering::Relaxed),
        }
    }
}

static REMOVE_NODE_LINK_COUNTERS: SiteCounters = SiteCounters::new();
static LINK_UNLINK_FROM_NODE_COUNTERS: SiteCounters = SiteCounters::new();
static SLINK_DESTRUCTOR_COUNTERS: SiteCounters = SiteCounters::new();

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
}

/// Aggregate counters for every site replaced by this module.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkRemovalSnapshot {
    pub remove_node_link: SiteSnapshot,
    pub link_unlink_from_node: SiteSnapshot,
    pub slink_destructor: SiteSnapshot,
}

/// Reads the aggregate counters without touching game memory.
pub fn snapshot() -> LinkRemovalSnapshot {
    LinkRemovalSnapshot {
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

/// Releases a 0x18-byte registry cell through the game's own free wrapper.
///
/// # Safety
///
/// `cell` must be a cell that the caller has already unlinked from its list and
/// that no other native object references. The native wrapper's size argument
/// is always [`CELL_BYTES`]; this must run on the game thread so the game
/// allocator sees the same thread as native removal.
#[inline]
unsafe fn free_cell(cell: *mut u8) {
    let free: unsafe extern "C" fn(*mut u8, usize) = transmute(NATIVE_FREE_ADDRESS);
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
            };
        }
        cell = read_pointer(cell, CELL_NEXT);
    }

    SearchResult {
        cell: std::ptr::null_mut(),
        hops,
        tail_probed: false,
        tail_hit: false,
    }
}

/// Records one completed search without formatting or logging.
#[inline]
fn record_search(counters: &SiteCounters, search: &SearchResult) {
    counters.calls.fetch_add(1, Ordering::Relaxed);
    if search.hops != 0 {
        counters
            .search_hops
            .fetch_add(search.hops, Ordering::Relaxed);
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

/// Whole-body replacement for `RemoveNodeLink(Node*, Link*) -> bool`.
///
/// # Safety
///
/// Entered by the installed jump with the native Windows x64 argument registers:
/// `node` is `RCX` (a live `Node`) and `link` is `RDX` (the `Link*` to remove).
/// It returns `true` exactly when a cell was unlinked and freed. A null `node`
/// is rejected defensively where the native body would fault; valid callers
/// never pass one.
#[inline(never)]
unsafe extern "C" fn remove_node_link(node: *mut u8, link: *mut u8) -> bool {
    if node.is_null() {
        REMOVE_NODE_LINK_COUNTERS
            .calls
            .fetch_add(1, Ordering::Relaxed);
        return false;
    }

    let tail_probe = TAIL_PROBE.load(Ordering::Relaxed);
    let result = search(node, link, tail_probe);
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
unsafe extern "C" fn link_unlink_from_node(link: *mut u8) {
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
    let result = search(node, link, tail_probe);
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
unsafe extern "C" fn slink_scalar_deleting_destructor(
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
        let result = search(node, link, tail_probe);
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

    let base_destructor: unsafe extern "C" fn(*mut u8) = transmute(SLINK_BASE_DESTRUCTOR_ADDRESS);
    base_destructor(this);

    if deleting_flag & 1 != 0 {
        let free: unsafe extern "C" fn(*mut u8, usize) = transmute(NATIVE_FREE_ADDRESS);
        free(this, SLINK_BYTES);
    }

    this
}

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
/// replays a relocated prologue and no RIP-relative instruction is copied. The
/// trampoline's `jmp [rip]` reaches the Rust body regardless of where the
/// injected DLL was mapped.
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

/// Validates the three pinned native bodies and prepares whole-body replacement
/// detours in `session`.
///
/// `tail_probe` enables the newest-cell test on lists of at least
/// [`TAIL_PROBE_MIN_LENGTH`] entries. With it disabled the replacements are
/// still equivalent to native (and still collect counters), but provide no
/// search-position advantage.
///
/// No patch is added unless every native body matches. The session remains
/// droppable without touching the process; installation is the parent's
/// permanent step and must run on the game thread.
///
/// # Safety
///
/// The process image must be the supported Steam 1.163 executable at its
/// preferred base. The caller must install and keep this DLL resident for the
/// process lifetime, and no other module may own the same three entry points.
/// All three routines must remain game-thread-only, as native code assumes.
pub unsafe fn prepare(session: &mut PatchSession, tail_probe: bool) -> Result<(), String> {
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
    Ok(())
}
