//! Bounded name index for the animation media bucket.
//!
//! `CopyNameIntoMedia` (`0x140178950`) selects one of 13 singly linked buckets
//! and compares names byte by byte until the first mismatch. The installed
//! animation bucket holds 2249 descriptors, so a lookup walks roughly a
//! thousand dependent loads; the live v0.1.4 observer measured 299 290 497
//! hops for 209 713 animation calls (99.18% of all media-lookup hops).
//!
//! The index replaces only the three native CALL sites that look up **type 8**
//! (animation): the wrapper at `0x140170241` and the two map-decoration sites at
//! `0x14015d1ff` / `0x14015d226`. On a verified hit it returns the same
//! descriptor the native scan would return, without walking the chain. Every
//! unsupported case (other types, unknown media, full name buffers, lock
//! contention, a disabled index, or a miss) calls the untouched native function
//! and returns its result. The native body is never patched.
//!
//! # Why a name index is lifetime-safe here
//!
//! The proof obligations and the evidence for each:
//!
//! 1. **One media manager, created once.** `MediaClassConstructor` (`0x14016fd30`)
//!    has one caller, `Init_HGE` (`0x1401ad080`); `Init_HGE` has one caller,
//!    `PrepareMainLoop` (`0x1401ad5a0`), called once from `Main`. The singleton
//!    `0x143a119e8` has exactly one writer, `0x1401ad342` in `Init_HGE`.
//! 2. **Animation descriptors are created only at startup.** The type-8 bucket
//!    head (`media+0x50`) is written by exactly two instructions in the media
//!    subsystem: the constructor zeroes it (`0x14016fd9f`) and
//!    `ParseResourceFile?` head-inserts a parsed descriptor (`0x14017add8`).
//!    `ParseResourceFile?` has one external caller, `MediaClassConstructor`
//!    (`0x14016fdc3`), plus its own `Include` recursion. Nothing re-parses
//!    resources during play.
//! 3. **Nodes are not freed during play.** The animation deleting destructor
//!    (`0x1401787a0`) is reachable only through its vtable slot 0; the vtable
//!    (`0x140366ca0`) is written only by the type-8 parse branch (`0x14017acc9`).
//!    `Resource_ClearHashLists` (`0x140170ae0`) is the only bucket walker that
//!    destroys nodes; it has one caller, `Media.resDeconstructor`
//!    (`0x1401700b0`), which has one caller, the shutdown tail of
//!    `PrepareMainLoop` (`0x1401adf3c`). The constructor's other destroy loop
//!    only clears bucket 0 before the manager is used.
//! 4. **Published nodes are not mutated or unlinked.** The only writers of a
//!    node's `next` (`+0x98`) are the parse-time head insertions and the
//!    field-copy helpers, and every field-copy destination is a freshly
//!    allocated descriptor. No code rewrites a predecessor's `next`, so an
//!    interior node cannot be removed. The name at `+8` is written by the same
//!    parse-time copy loops into the fresh descriptor.
//! 5. **Duplicate names exist and the newest wins.** The shipped data contains
//!    two duplicate animation names (`ask_for_help`, `ask_for_help_eng` in
//!    `Static2.res` and `Mapmode3.res`). On a name hit both parse paths copy the
//!    old fields into the new descriptor and still head-insert it, so the bucket
//!    can contain duplicates and the native scan returns the head-most one. The
//!    index records the node native actually returned and never invents an
//!    older duplicate.
//! 6. **Any insertion or clear invalidates the index.** Lookups compare the
//!    current bucket head with the head observed when the index was populated.
//!    A push changes the head and a clear sets it to null; either mismatch
//!    flushes the table before it is used. This is defence in depth on top of
//!    obligations 2 and 3, and it makes a hypothetical late insertion safe.
//! 7. **Virtual destruction is intercepted before storage release.** The type-8
//!    vtable's scalar deleting-destructor slot (`0x140366ca0`) delegates through
//!    a wrapper to the untouched `0x1401787a0`. Before calling native, it
//!    permanently disables the index. Thus even a virtual delete outside the
//!    expected shutdown clear loop cannot leave a subsequently usable handle.
//!    Only the published singleton manager is eligible for acceleration.
//!
//! Residual risk, stated rather than hidden: obligations 2, 3 and 4 are xref and
//! instruction-census results, so a writer that is not reachable from the
//! audited media code and does not appear in the 0x140160000..0x140180000
//! instruction census could still invalidate a node. The index therefore keeps
//! a sampled full-native verification path; a semantic disagreement disables
//! the accelerator. Nothing here can prove memory safety for an unlocated
//! mutation, and the sampler is not a lifetime proof.
//!
//! # What is deliberately not done
//!
//! * No negative cache. Proving that a name is absent would require a scan;
//!   keying an "absent" answer by a 64-bit hash would make correctness depend on
//!   hash quality, because a collision would turn a present name into a miss.
//! * No pointer validation by dereference. Hits are checked against the
//!   descriptor name with native equality semantics, but that check is a
//!   correctness check, not a validity test for freed storage.
//! * No owning copy of a game string or descriptor, and no allocation.
//!
//! # Threading
//!
//! Every table access happens under a try-lock. The lock is never waited on: a
//! call that finds it busy takes the native path, so contention can only reduce
//! the hit rate. This protects the index arrays; it does **not** make the
//! native media lists thread-safe, and the game's own publication of a node is
//! not synchronised by this module.
//!
//! # Observation mode
//!
//! An explicit measurement mode retains the v0.1.4 shadow scan at all seven
//! native CALL sites. It is off unless `probe_enabled` is passed to
//! [`prepare`], and it takes precedence over the index. Its counters are
//! evidence about the native chain, not an optimisation measurement.

use std::{
    cell::UnsafeCell,
    mem::transmute,
    slice,
    sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};

use patchy::PatchSession;

use crate::call_hook;

const COPY_NAME_INTO_MEDIA: usize = 0x1_4017_8950;
const MEDIA_SINGLETON: usize = 0x1_43a1_19e8;
const ANIMATION_DELETING_DTOR_SLOT: usize = 0x1_4036_6ca0;
const ANIMATION_DELETING_DTOR: usize = 0x1_4017_87a0;
const ANIMATION_DELETING_DTOR_BODY: [u8; 70] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x8b, 0xda, 0x48, 0x8b, 0xf9, 0x48,
    0x8d, 0x05, 0x9a, 0xde, 0x1e, 0x00, 0x48, 0x89, 0x01, 0x48, 0x8b, 0x0d, 0xd0, 0xfe, 0x86, 0x03,
    0x48, 0x8b, 0x01, 0xff, 0x10, 0x90, 0xf6, 0xc3, 0x01, 0x74, 0x0d, 0xba, 0x68, 0x01, 0x00, 0x00,
    0x48, 0x8b, 0xcf, 0xe8, 0xf8, 0xb8, 0x1b, 0x00, 0x48, 0x8b, 0xc7, 0x48, 0x8b, 0x5c, 0x24, 0x30,
    0x48, 0x83, 0xc4, 0x20, 0x5f, 0xc3,
];
const COPY_NAME_INTO_MEDIA_BODY: [u8; 74] = [
    0x48, 0x63, 0xc2, 0x4c, 0x8b, 0x54, 0xc1, 0x10, 0x4d, 0x85, 0xd2, 0x74, 0x36, 0x0f, 0x1f, 0x00,
    0x4d, 0x8d, 0x4a, 0x08, 0x49, 0x8b, 0xc0, 0x4d, 0x2b, 0xc8, 0x66, 0x0f, 0x1f, 0x44, 0x00, 0x00,
    0x0f, 0xb6, 0x10, 0x42, 0x0f, 0xb6, 0x0c, 0x08, 0x2b, 0xd1, 0x75, 0x07, 0x48, 0xff, 0xc0, 0x85,
    0xc9, 0x75, 0xed, 0x85, 0xd2, 0x74, 0x0f, 0x4d, 0x8b, 0x92, 0x98, 0x00, 0x00, 0x00, 0x4d, 0x85,
    0xd2, 0x75, 0xcd, 0x33, 0xc0, 0xc3, 0x49, 0x8b, 0xc2, 0xc3,
];

const ANIMATION_TYPE: i32 = 8;
const BUCKET_HEADS: usize = 0x10;
/// `media + 0x10 + type*8` for the animation bucket.
const ANIMATION_BUCKET: usize = BUCKET_HEADS + ANIMATION_TYPE as usize * 8;
const DESCRIPTOR_NAME: usize = 0x08;
const DESCRIPTOR_NEXT: usize = 0x98;

/// `ResDesc_CopyBaseFields` copies exactly `+0x8..+0x88`, so the name area is
/// 128 bytes. A lookup whose name does not terminate inside it is not indexed.
const MAX_INDEXED_NAME_BYTES: usize = 0x80;

const SET_BITS: usize = 11;
const SETS: usize = 1 << SET_BITS;
const WAYS: usize = 4;
const SLOTS: usize = SETS * WAYS;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const SET_MIX: u64 = 0x9E37_79B9_7F4A_7C15;

// --- index -----------------------------------------------------------------------------------

struct IndexState {
    keys: [u64; SLOTS],
    nodes: [usize; SLOTS],
    media: usize,
    head: usize,
    initialised: bool,
}

struct NodeIndex {
    state: UnsafeCell<IndexState>,
    lock: AtomicBool,
}

// SAFETY: every array access happens under `lock`; the diagnostic thread reads
// only the atomics in the counter structs, never this state.
unsafe impl Sync for NodeIndex {}

static INDEX: NodeIndex = NodeIndex {
    state: UnsafeCell::new(IndexState {
        keys: [0; SLOTS],
        nodes: [0; SLOTS],
        media: 0,
        head: 0,
        initialised: false,
    }),
    lock: AtomicBool::new(false),
};

static INDEX_ENABLED: AtomicBool = AtomicBool::new(false);
static INDEX_AUTO_DISABLED: AtomicBool = AtomicBool::new(false);
static LIFETIME_DISABLED: AtomicBool = AtomicBool::new(false);
static FOREIGN_MEDIA: AtomicU64 = AtomicU64::new(0);
static DESTRUCTOR_CALLS: AtomicU64 = AtomicU64::new(0);
static PROBE_ACTIVE: AtomicBool = AtomicBool::new(false);
static VERIFY_SHIFT: AtomicU32 = AtomicU32::new(0);
static VERIFY_MASK: AtomicU64 = AtomicU64::new(0);

struct LockGuard;

impl LockGuard {
    fn acquire() -> Option<Self> {
        if INDEX
            .lock
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(Self)
        } else {
            None
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        INDEX.lock.store(false, Ordering::Release);
    }
}

fn set_base(key: u64) -> usize {
    // Fibonacci mixing; heap-free name hashes differ in every bit.
    ((key.wrapping_mul(SET_MIX) >> (64 - SET_BITS)) as usize) * WAYS
}

fn eviction_way(key: u64) -> usize {
    ((key.wrapping_mul(SET_MIX) >> SET_BITS) as usize) & (WAYS - 1)
}

/// Reads the animation bucket head exactly as the native body does.
///
/// # Safety
///
/// `media` must be the live manager pointer supplied to `CopyNameIntoMedia`.
/// This helper intentionally does not test the pointer or the index.
unsafe fn bucket_head(media: *mut u8) -> usize {
    (media.add(ANIMATION_BUCKET) as *const usize).read_unaligned()
}

/// Hashes a NUL-terminated name. Returns `None` when no NUL appears inside the
/// descriptor name area, in which case the caller uses the native lookup.
///
/// # Safety
///
/// `name` must be readable up to its terminator, as the native API already
/// requires. The loop reads at most [`MAX_INDEXED_NAME_BYTES`] bytes.
unsafe fn name_key(name: *const u8) -> Option<u64> {
    let mut hash = FNV_OFFSET;

    for offset in 0..MAX_INDEXED_NAME_BYTES {
        // SAFETY: Native callers pass a readable NUL-terminated name; the loop
        // bounds the read even when that contract is violated.
        let byte = unsafe { name.add(offset).read() };
        if byte == 0 {
            return Some(hash | 1);
        }
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    None
}

/// Native equality: byte-wise, case-sensitive, and terminating at the shared
/// NUL. The stored name is read only from the 128-byte descriptor field.
///
/// # Safety
///
/// `node` must be a live type-8 descriptor and `name` readable up to its
/// terminator. Both reads are bounded.
unsafe fn name_matches(node: usize, name: *const u8) -> bool {
    let stored = (node as *const u8).add(DESCRIPTOR_NAME);

    for offset in 0..MAX_INDEXED_NAME_BYTES {
        // SAFETY: See above; the descriptor name area is 128 bytes.
        let query = unsafe { name.add(offset).read() };
        let entry = unsafe { stored.add(offset).read() };
        if query != entry {
            return false;
        }
        if query == 0 {
            return true;
        }
    }

    false
}

/// Returns every table entry whose key matches and whose stored name equals the
/// query. All [`WAYS`] slots are examined because eviction can leave a hole.
///
/// # Safety
///
/// `name` must satisfy [`name_matches`]. The table lock must be held.
unsafe fn probe_locked(state: &IndexState, key: u64, name: *const u8) -> Option<usize> {
    let base = set_base(key);

    for way in 0..WAYS {
        let slot = base + way;
        if state.keys[slot] != key {
            continue;
        }
        let node = state.nodes[slot];
        // SAFETY: Caller contract and the module-level lifecycle proof.
        if unsafe { name_matches(node, name) } {
            return Some(node);
        }
        // A collision or a stale name: fall through to the next way and, if no
        // way matches, let the native scan decide.
        TOTALS.collisions.fetch_add(1, Ordering::Relaxed);
    }

    None
}

/// Records the node the native lookup returned. A same-key slot is overwritten
/// because the native result is authoritative.
///
/// The table lock must be held.
fn insert_locked(state: &mut IndexState, key: u64, node: usize) {
    let base = set_base(key);
    let mut empty = None;

    for way in 0..WAYS {
        let slot = base + way;
        if state.keys[slot] == key {
            state.nodes[slot] = node;
            TOTALS.inserts.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if state.keys[slot] == 0 && empty.is_none() {
            empty = Some(slot);
        }
    }

    let slot = match empty {
        Some(slot) => slot,
        None => {
            TOTALS.evictions.fetch_add(1, Ordering::Relaxed);
            base + eviction_way(key)
        }
    };
    state.keys[slot] = key;
    state.nodes[slot] = node;
    TOTALS.inserts.fetch_add(1, Ordering::Relaxed);
}

/// Adopts a `(media, head)` generation, flushing every entry when the manager or
/// the bucket head changed. Insertion pushes at the head and a clear nulls it,
/// so either change invalidates every recorded node.
///
/// The table lock must be held.
fn adopt_generation(state: &mut IndexState, media: usize, head: usize) {
    if state.initialised && state.media == media && state.head == head {
        return;
    }
    if state.initialised {
        TOTALS.epoch_changes.fetch_add(1, Ordering::Relaxed);
    }
    state.keys.fill(0);
    state.nodes.fill(0);
    state.media = media;
    state.head = head;
    state.initialised = true;
}

// --- counters --------------------------------------------------------------------------------

struct SiteCounters {
    calls: AtomicU64,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl SiteCounters {
    const fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }
}

static SITES: [SiteCounters; 3] = [
    SiteCounters::new(),
    SiteCounters::new(),
    SiteCounters::new(),
];

struct Totals {
    native_calls: AtomicU64,
    busy: AtomicU64,
    foreign_type: AtomicU64,
    capped_names: AtomicU64,
    collisions: AtomicU64,
    verify_samples: AtomicU64,
    verify_mismatches: AtomicU64,
    inserts: AtomicU64,
    evictions: AtomicU64,
    epoch_changes: AtomicU64,
}

impl Totals {
    const fn new() -> Self {
        Self {
            native_calls: AtomicU64::new(0),
            busy: AtomicU64::new(0),
            foreign_type: AtomicU64::new(0),
            capped_names: AtomicU64::new(0),
            collisions: AtomicU64::new(0),
            verify_samples: AtomicU64::new(0),
            verify_mismatches: AtomicU64::new(0),
            inserts: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
            epoch_changes: AtomicU64::new(0),
        }
    }
}

static TOTALS: Totals = Totals::new();

// --- replacement bodies ----------------------------------------------------------------------

/// Calls the untouched native lookup.
///
/// # Safety
///
/// The caller supplies the exact native three-argument CALL contract.
unsafe fn call_native(media: *mut u8, resource_type: i32, name: *const u8) -> *mut u8 {
    // SAFETY: Address pinned and byte-guarded by `prepare`.
    let lookup: unsafe extern "C" fn(*mut u8, i32, *const u8) -> *mut u8 =
        unsafe { transmute(COPY_NAME_INTO_MEDIA) };
    unsafe { lookup(media, resource_type, name) }
}

/// Calls the native lookup and counts it as an index-path fallback.
///
/// # Safety
///
/// As [`call_native`].
unsafe fn call_native_indexed(media: *mut u8, resource_type: i32, name: *const u8) -> *mut u8 {
    TOTALS.native_calls.fetch_add(1, Ordering::Relaxed);
    // SAFETY: Arguments are forwarded unchanged.
    unsafe { call_native(media, resource_type, name) }
}

/// Invalidate before any virtual animation destruction, including unexpected
/// deletion outside shutdown. Never reproduce or omit a native destructor step.
unsafe extern "C-unwind" fn animation_deleting_destructor(
    descriptor: *mut u8,
    flags: u32,
) -> *mut u8 {
    INDEX_ENABLED.store(false, Ordering::Release);
    LIFETIME_DISABLED.store(true, Ordering::Relaxed);
    DESTRUCTOR_CALLS.fetch_add(1, Ordering::Relaxed);
    // SAFETY: Exact scalar-deleting ABI from the guarded native body: RCX=this,
    // EDX=flags, RAX=this. Native retains HGE release/free and unwind behavior.
    let native: unsafe extern "C-unwind" fn(*mut u8, u32) -> *mut u8 =
        unsafe { transmute(ANIMATION_DELETING_DTOR) };
    unsafe { native(descriptor, flags) }
}

/// Indexed animation lookup. Returns exactly what the native lookup would.
///
/// # Safety
///
/// Entered through a validated native CALL relay with the original `media`,
/// `type` and `name` arguments. Those must satisfy `CopyNameIntoMedia`'s
/// contract.
unsafe fn run_index(media: *mut u8, resource_type: i32, name: *const u8, site: usize) -> *mut u8 {
    let counters = &SITES[site];
    counters.calls.fetch_add(1, Ordering::Relaxed);

    if !INDEX_ENABLED.load(Ordering::Acquire) || media.is_null() {
        // SAFETY: Arguments are forwarded unchanged.
        return unsafe { call_native_indexed(media, resource_type, name) };
    }
    if resource_type != ANIMATION_TYPE {
        // The three patched sites always pass 8; this keeps a reused site safe.
        TOTALS.foreign_type.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Arguments are forwarded unchanged.
        return unsafe { call_native_indexed(media, resource_type, name) };
    }
    // SAFETY: Fixed mapped singleton slot in the supported executable. The
    // manager is published only after its constructor/startup parse completes.
    let singleton = unsafe { (MEDIA_SINGLETON as *const usize).read_unaligned() };
    if media as usize != singleton {
        FOREIGN_MEDIA.fetch_add(1, Ordering::Relaxed);
        return unsafe { call_native_indexed(media, resource_type, name) };
    }

    // SAFETY: Native lookup contract; bounded to the descriptor name area.
    let Some(key) = (unsafe { name_key(name) }) else {
        TOTALS.capped_names.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Arguments are forwarded unchanged.
        return unsafe { call_native_indexed(media, resource_type, name) };
    };

    let Some(guard) = LockGuard::acquire() else {
        TOTALS.busy.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Arguments are forwarded unchanged.
        return unsafe { call_native_indexed(media, resource_type, name) };
    };

    // SAFETY: The caller contract covers the media pointer; the same unchecked
    // offset arithmetic as the native body is used.
    let head = unsafe { bucket_head(media) };
    // SAFETY: The lock is held, so this is the only reference to the state.
    let state = unsafe { &mut *INDEX.state.get() };
    adopt_generation(state, media as usize, head);
    // SAFETY: Table lock held; `name` satisfies the native contract.
    let hit = unsafe { probe_locked(state, key, name) };
    drop(guard);

    if let Some(node) = hit {
        let hits = counters.hits.fetch_add(1, Ordering::Relaxed) + 1;
        let mask = VERIFY_MASK.load(Ordering::Relaxed);

        // Sampled full-native verification: one hit in 2^shift. It can detect a
        // semantic disagreement but cannot prove that a pointer is not stale.
        if mask != 0 && (hits & mask) == 0 {
            TOTALS.verify_samples.fetch_add(1, Ordering::Relaxed);
            // SAFETY: Arguments are forwarded unchanged.
            let native = unsafe { call_native_indexed(media, resource_type, name) };
            if native as usize != node {
                TOTALS.verify_mismatches.fetch_add(1, Ordering::Relaxed);
                // One disagreement falsifies the semantic contract. Do not
                // tolerate known wrong results between verification samples.
                INDEX_AUTO_DISABLED.store(true, Ordering::Relaxed);
                INDEX_ENABLED.store(false, Ordering::Release);
            }
            // Native is authoritative on every sampled call.
            return native;
        }

        return node as *mut u8;
    }

    counters.misses.fetch_add(1, Ordering::Relaxed);
    // SAFETY: Arguments are forwarded unchanged.
    let native = unsafe { call_native_indexed(media, resource_type, name) };
    if native.is_null() {
        return native;
    }

    // Populate only from an authoritative native hit, and only while the
    // generation that produced it is still current.
    if let Some(guard) = LockGuard::acquire() {
        // SAFETY: Table lock held for the whole block.
        let state = unsafe { &mut *INDEX.state.get() };
        // SAFETY: Same media pointer contract as the lookup above.
        let head_now = unsafe { bucket_head(media) };
        if head_now == head
            && state.initialised
            && state.media == media as usize
            && state.head == head
        {
            insert_locked(state, key, native as usize);
        } else {
            adopt_generation(state, media as usize, head_now);
        }
        drop(guard);
    } else {
        TOTALS.busy.fetch_add(1, Ordering::Relaxed);
    }

    native
}

macro_rules! index_wrapper {
    ($function:ident, $site:expr) => {
        unsafe extern "C" fn $function(
            media: *mut u8,
            resource_type: i32,
            name: *const u8,
        ) -> *mut u8 {
            // SAFETY: The relay preserves the validated native CALL ABI.
            unsafe { run_index(media, resource_type, name, $site) }
        }
    };
}

index_wrapper!(animation_index, 0);
index_wrapper!(map_animation_a_index, 1);
index_wrapper!(map_animation_b_index, 2);

// --- measurement-only shadow probe (explicitly enabled) --------------------------------------

/// The shipped animation bucket has 2249 entries. This cap leaves room for
/// runtime growth while ensuring that a corrupt list cannot make the observer
/// walk forever. Native lookup still runs after a capped shadow scan.
const MAX_SHADOW_HOPS: u64 = 4096;
const MAX_PROBE_NAME_BYTES: usize = 4096;

const NAME_HASH_SLOTS: usize = 1 << 16;
const NAME_HASH_PROBES: usize = 16;
static NAME_HASHES: [AtomicU64; NAME_HASH_SLOTS] = [const { AtomicU64::new(0) }; NAME_HASH_SLOTS];
static DISTINCT_NAMES: AtomicU64 = AtomicU64::new(0);
static NAME_HASH_FULL: AtomicU64 = AtomicU64::new(0);
static NAME_HASH_CAPPED: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
struct CallSite {
    name: &'static str,
    address: usize,
    call: [u8; 5],
    replacement: *const (),
}

// Function pointers are just addresses used to construct native relays.
unsafe impl Sync for CallSite {}

const INDEX_SITES: [CallSite; 3] = [
    CallSite {
        name: "animation",
        address: 0x1_4017_0241,
        call: [0xe8, 0x0a, 0x87, 0x00, 0x00],
        replacement: animation_index as *const (),
    },
    CallSite {
        name: "map animation A",
        address: 0x1_4015_d1ff,
        call: [0xe8, 0x4c, 0xb7, 0x01, 0x00],
        replacement: map_animation_a_index as *const (),
    },
    CallSite {
        name: "map animation B",
        address: 0x1_4015_d226,
        call: [0xe8, 0x25, 0xb7, 0x01, 0x00],
        replacement: map_animation_b_index as *const (),
    },
];

const TYPE_11: usize = 1;
const SOUND: usize = 2;
const FONT: usize = 3;
const TEXTURE: usize = 4;

const PROBE_SITES: [CallSite; 7] = [
    CallSite {
        name: "animation",
        address: 0x1_4017_0241,
        call: [0xe8, 0x0a, 0x87, 0x00, 0x00],
        replacement: animation_probe as *const (),
    },
    CallSite {
        name: "type11",
        address: 0x1_4017_0281,
        call: [0xe8, 0xca, 0x86, 0x00, 0x00],
        replacement: type_11_probe as *const (),
    },
    CallSite {
        name: "sound",
        address: 0x1_4017_02ca,
        call: [0xe8, 0x81, 0x86, 0x00, 0x00],
        replacement: sound_probe as *const (),
    },
    CallSite {
        name: "font",
        address: 0x1_4017_03f1,
        call: [0xe8, 0x5a, 0x85, 0x00, 0x00],
        replacement: font_probe as *const (),
    },
    CallSite {
        name: "texture",
        address: 0x1_4017_0460,
        call: [0xe8, 0xeb, 0x84, 0x00, 0x00],
        replacement: texture_probe as *const (),
    },
    CallSite {
        name: "map animation A",
        address: 0x1_4015_d1ff,
        call: [0xe8, 0x4c, 0xb7, 0x01, 0x00],
        replacement: map_animation_a_probe as *const (),
    },
    CallSite {
        name: "map animation B",
        address: 0x1_4015_d226,
        call: [0xe8, 0x25, 0xb7, 0x01, 0x00],
        replacement: map_animation_b_probe as *const (),
    },
];

struct ProbeCounters {
    calls: AtomicU64,
    hits: AtomicU64,
    misses: AtomicU64,
    hops: AtomicU64,
    compare_bytes: AtomicU64,
    max_hops: AtomicU64,
    capped: AtomicU64,
    mismatches: AtomicU64,
}

impl ProbeCounters {
    const fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            hops: AtomicU64::new(0),
            compare_bytes: AtomicU64::new(0),
            max_hops: AtomicU64::new(0),
            capped: AtomicU64::new(0),
            mismatches: AtomicU64::new(0),
        }
    }
}

static PROBE_COUNTERS: [ProbeCounters; 7] = [
    ProbeCounters::new(),
    ProbeCounters::new(),
    ProbeCounters::new(),
    ProbeCounters::new(),
    ProbeCounters::new(),
    ProbeCounters::new(),
    ProbeCounters::new(),
];

struct Observation {
    predicted: *mut u8,
    hops: u64,
    compare_bytes: u64,
    complete: bool,
}

/// Reproduces the native search as a read-only observer.
///
/// # Safety
/// The native caller must supply the same live media manager and NUL-terminated
/// name required by `CopyNameIntoMedia`. Bucket arithmetic deliberately keeps
/// the native sign-extension and wrapping behavior; this is not a validator.
unsafe fn observe_lookup(media: *mut u8, resource_type: i32, name: *const u8) -> Observation {
    let bucket_offset = BUCKET_HEADS.wrapping_add((resource_type as i64 as usize).wrapping_mul(8));
    let bucket = (media as usize).wrapping_add(bucket_offset) as *const usize;
    // SAFETY: Same pointer arithmetic the native body performs.
    let mut node = unsafe { bucket.read_unaligned() } as *mut u8;
    let mut hops = 0u64;
    let mut compare_bytes = 0u64;

    while !node.is_null() && hops < MAX_SHADOW_HOPS {
        hops += 1;
        let entry_name = node.wrapping_add(DESCRIPTOR_NAME);
        let mut offset = 0usize;

        loop {
            if offset == MAX_PROBE_NAME_BYTES {
                return Observation {
                    predicted: std::ptr::null_mut(),
                    hops,
                    compare_bytes,
                    complete: false,
                };
            }
            // SAFETY: Native requires both strings to be readable and
            // NUL-terminated; the hop cap bounds the walk.
            let query_byte = unsafe { name.add(offset).read() };
            let entry_byte = unsafe { entry_name.add(offset).read() };
            compare_bytes += 1;
            if query_byte != entry_byte {
                break;
            }
            if entry_byte == 0 {
                return Observation {
                    predicted: node,
                    hops,
                    compare_bytes,
                    complete: true,
                };
            }
            offset += 1;
        }

        // SAFETY: Chain nodes are readable while native lookup is valid.
        node = unsafe { (node.add(DESCRIPTOR_NEXT) as *const usize).read_unaligned() } as *mut u8;
    }

    Observation {
        predicted: std::ptr::null_mut(),
        hops,
        compare_bytes,
        complete: node.is_null(),
    }
}

/// Hashes the complete API name, not the native descriptor name. Hash collisions
/// only make the distinct-count estimate less precise; they cannot affect a
/// lookup result. Sixteen atomic probes bound concurrent observer maintenance.
fn record_name(resource_type: i32, name: *const u8) {
    let mut hash = FNV_OFFSET ^ (resource_type as u32 as u64);
    let mut terminated = false;

    // SAFETY: The native API requires a readable NUL-terminated name. The cap
    // additionally prevents an invalid name from causing unbounded observer work.
    for offset in 0..MAX_PROBE_NAME_BYTES {
        let byte = unsafe { name.add(offset).read() };
        if byte == 0 {
            terminated = true;
            break;
        }
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    if !terminated {
        NAME_HASH_CAPPED.fetch_add(1, Ordering::Relaxed);
        return;
    }

    let key = hash | 1;
    let base = key as usize & (NAME_HASH_SLOTS - 1);
    for probe in 0..NAME_HASH_PROBES {
        let slot = &NAME_HASHES[(base + probe) & (NAME_HASH_SLOTS - 1)];
        let found = slot.load(Ordering::Relaxed);
        if found == key {
            return;
        }
        if found == 0 {
            match slot.compare_exchange(0, key, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => {
                    DISTINCT_NAMES.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                Err(current) if current == key => return,
                Err(_) => continue,
            }
        }
    }
    NAME_HASH_FULL.fetch_add(1, Ordering::Relaxed);
}

/// Shared observer body. The native lookup remains the sole authority for the
/// return value, including when the bounded shadow scan was incomplete.
unsafe fn run_probe(media: *mut u8, resource_type: i32, name: *const u8, site: usize) -> *mut u8 {
    // SAFETY: Caller contract is the native lookup contract.
    let observation = unsafe { observe_lookup(media, resource_type, name) };
    record_name(resource_type, name);

    // SAFETY: Arguments are forwarded unchanged.
    let native_result = unsafe { call_native(media, resource_type, name) };
    let counters = &PROBE_COUNTERS[site];

    counters.calls.fetch_add(1, Ordering::Relaxed);
    counters.hops.fetch_add(observation.hops, Ordering::Relaxed);
    counters
        .compare_bytes
        .fetch_add(observation.compare_bytes, Ordering::Relaxed);
    counters
        .max_hops
        .fetch_max(observation.hops, Ordering::Relaxed);

    if native_result.is_null() {
        counters.misses.fetch_add(1, Ordering::Relaxed);
    } else {
        counters.hits.fetch_add(1, Ordering::Relaxed);
    }
    if !observation.complete {
        counters.capped.fetch_add(1, Ordering::Relaxed);
    } else if observation.predicted != native_result {
        counters.mismatches.fetch_add(1, Ordering::Relaxed);
    }

    native_result
}

macro_rules! probe_wrapper {
    ($function:ident, $site:expr) => {
        unsafe extern "C" fn $function(
            media: *mut u8,
            resource_type: i32,
            name: *const u8,
        ) -> *mut u8 {
            // SAFETY: The relay preserves the validated native CALL ABI.
            unsafe { run_probe(media, resource_type, name, $site) }
        }
    };
}

probe_wrapper!(animation_probe, 0);
probe_wrapper!(type_11_probe, TYPE_11);
probe_wrapper!(sound_probe, SOUND);
probe_wrapper!(font_probe, FONT);
probe_wrapper!(texture_probe, TEXTURE);
probe_wrapper!(map_animation_a_probe, 5);
probe_wrapper!(map_animation_b_probe, 6);

// --- snapshots ---------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IndexSiteSnapshot {
    pub calls: u64,
    pub hits: u64,
    pub misses: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IndexSnapshot {
    pub enabled: bool,
    pub auto_disabled: bool,
    pub lifetime_disabled: bool,
    pub destructor_calls: u64,
    pub foreign_media: u64,
    /// Effective sampled-verification shift; 0 means sampling is off.
    pub verify_shift: u32,
    pub hits: u64,
    pub misses: u64,
    pub native_calls: u64,
    pub busy: u64,
    pub foreign_type: u64,
    pub capped_names: u64,
    pub collisions: u64,
    pub verify_samples: u64,
    pub verify_mismatches: u64,
    pub inserts: u64,
    pub evictions: u64,
    pub epoch_changes: u64,
    pub sites: [IndexSiteSnapshot; 3],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProbeSiteSnapshot {
    pub calls: u64,
    pub hops: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProbeSnapshot {
    pub active: bool,
    pub calls: u64,
    pub hits: u64,
    pub misses: u64,
    pub hops: u64,
    pub compare_bytes: u64,
    pub max_hops: u64,
    pub capped: u64,
    pub mismatches: u64,
    pub distinct_names: u64,
    pub name_hash_full: u64,
    pub name_hash_capped: u64,
    pub sites: [ProbeSiteSnapshot; 7],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MediaSnapshot {
    pub index: IndexSnapshot,
    pub probe: ProbeSnapshot,
}

/// Reads atomics only; the diagnostic thread never walks native media lists.
pub fn snapshot() -> MediaSnapshot {
    let mut result = MediaSnapshot::default();

    let index = &mut result.index;
    index.enabled = INDEX_ENABLED.load(Ordering::Relaxed);
    index.auto_disabled = INDEX_AUTO_DISABLED.load(Ordering::Relaxed);
    index.lifetime_disabled = LIFETIME_DISABLED.load(Ordering::Relaxed);
    index.destructor_calls = DESTRUCTOR_CALLS.load(Ordering::Relaxed);
    index.foreign_media = FOREIGN_MEDIA.load(Ordering::Relaxed);
    index.verify_shift = VERIFY_SHIFT.load(Ordering::Relaxed);
    index.native_calls = TOTALS.native_calls.load(Ordering::Relaxed);
    index.busy = TOTALS.busy.load(Ordering::Relaxed);
    index.foreign_type = TOTALS.foreign_type.load(Ordering::Relaxed);
    index.capped_names = TOTALS.capped_names.load(Ordering::Relaxed);
    index.collisions = TOTALS.collisions.load(Ordering::Relaxed);
    index.verify_samples = TOTALS.verify_samples.load(Ordering::Relaxed);
    index.verify_mismatches = TOTALS.verify_mismatches.load(Ordering::Relaxed);
    index.inserts = TOTALS.inserts.load(Ordering::Relaxed);
    index.evictions = TOTALS.evictions.load(Ordering::Relaxed);
    index.epoch_changes = TOTALS.epoch_changes.load(Ordering::Relaxed);
    for (slot, counters) in SITES.iter().enumerate() {
        let calls = counters.calls.load(Ordering::Relaxed);
        let hits = counters.hits.load(Ordering::Relaxed);
        let misses = counters.misses.load(Ordering::Relaxed);
        index.sites[slot] = IndexSiteSnapshot {
            calls,
            hits,
            misses,
        };
        index.hits += hits;
        index.misses += misses;
    }

    let probe = &mut result.probe;
    probe.active = PROBE_ACTIVE.load(Ordering::Relaxed);
    probe.distinct_names = DISTINCT_NAMES.load(Ordering::Relaxed);
    probe.name_hash_full = NAME_HASH_FULL.load(Ordering::Relaxed);
    probe.name_hash_capped = NAME_HASH_CAPPED.load(Ordering::Relaxed);
    for (slot, counters) in PROBE_COUNTERS.iter().enumerate() {
        let calls = counters.calls.load(Ordering::Relaxed);
        let hops = counters.hops.load(Ordering::Relaxed);
        probe.sites[slot] = ProbeSiteSnapshot { calls, hops };
        probe.calls += calls;
        probe.hits += counters.hits.load(Ordering::Relaxed);
        probe.misses += counters.misses.load(Ordering::Relaxed);
        probe.hops += hops;
        probe.compare_bytes += counters.compare_bytes.load(Ordering::Relaxed);
        probe.max_hops = probe
            .max_hops
            .max(counters.max_hops.load(Ordering::Relaxed));
        probe.capped += counters.capped.load(Ordering::Relaxed);
        probe.mismatches += counters.mismatches.load(Ordering::Relaxed);
    }

    result
}

// --- preparation -----------------------------------------------------------------------------

/// One sampled native recheck per 1024 index hits.
pub const DEFAULT_VERIFY_SHIFT: u32 = 10;

/// Prepares the media lookup replacements with [`DEFAULT_VERIFY_SHIFT`].
///
/// `probe_enabled` selects the v0.1.4 observer instead of the index and takes
/// precedence: the observer needs all seven CALL sites, the index only the
/// three animation sites. With both flags false no patch is prepared.
///
/// # Safety
///
/// Runs once at quiescent Modloader initialization on the supported fixed-base
/// Steam 1.163 image. The DLL and all CALL relays must remain loaded for the
/// process lifetime, and no other module may own these CALL instructions.
pub unsafe fn prepare(
    session: &mut PatchSession,
    index_enabled: bool,
    probe_enabled: bool,
) -> Result<(), String> {
    INDEX_AUTO_DISABLED.store(false, Ordering::Relaxed);
    PROBE_ACTIVE.store(false, Ordering::Relaxed);

    if !index_enabled && !probe_enabled {
        INDEX_ENABLED.store(false, Ordering::Relaxed);
        return Ok(());
    }

    // SAFETY: The parent establishes the supported executable mapping before
    // any patch is prepared; this is the same body the native relay calls.
    let native_body = unsafe {
        slice::from_raw_parts(
            COPY_NAME_INTO_MEDIA as *const u8,
            COPY_NAME_INTO_MEDIA_BODY.len(),
        )
    };
    if native_body != COPY_NAME_INTO_MEDIA_BODY {
        INDEX_ENABLED.store(false, Ordering::Relaxed);
        return Err("media: CopyNameIntoMedia body differs at 0x140178950".into());
    }
    if index_enabled && !probe_enabled {
        // SAFETY: Supported mapped native vtable/function. Validate the entire
        // destructor and original slot before preparing any index hook.
        let slot = unsafe { (ANIMATION_DELETING_DTOR_SLOT as *const usize).read_unaligned() };
        let body = unsafe {
            slice::from_raw_parts(
                ANIMATION_DELETING_DTOR as *const u8,
                ANIMATION_DELETING_DTOR_BODY.len(),
            )
        };
        if slot != ANIMATION_DELETING_DTOR || body != ANIMATION_DELETING_DTOR_BODY {
            return Err("media: animation deleting-destructor contract differs".into());
        }
    }

    let sites: &[CallSite] = if probe_enabled {
        &PROBE_SITES
    } else {
        &INDEX_SITES
    };

    for site in sites {
        // SAFETY: Same supported-image contract; five bytes cover one native CALL.
        let actual = unsafe { slice::from_raw_parts(site.address as *const u8, 5) };
        if actual != site.call {
            INDEX_ENABLED.store(false, Ordering::Relaxed);
            return Err(format!(
                "media: {} CALL differs at {:#x}",
                site.name, site.address
            ));
        }
    }

    for site in sites {
        // SAFETY: Every replacement forwards the exact validated three-argument
        // CALL ABI and preserves the native return address through call_hook.
        unsafe {
            call_hook::prepare_call(session, site.address, &site.call, site.replacement)?;
        }
    }
    if index_enabled && !probe_enabled {
        let replacement = (animation_deleting_destructor as *const () as usize).to_le_bytes();
        // SAFETY: Quiescent startup, exact validated vtable slot. The wrapper
        // calls the original body once; no game object layout or owner changes.
        unsafe { session.overwrite(ANIMATION_DELETING_DTOR_SLOT, &replacement) }
            .map_err(|error| error.to_string())?;
    }

    VERIFY_SHIFT.store(DEFAULT_VERIFY_SHIFT, Ordering::Relaxed);
    VERIFY_MASK.store((1u64 << DEFAULT_VERIFY_SHIFT) - 1, Ordering::Relaxed);
    PROBE_ACTIVE.store(probe_enabled, Ordering::Relaxed);
    // Enabled last: installation follows this call, so no wrapper can observe a
    // half-initialised index. A dropped session leaves no relay and no calls.
    INDEX_ENABLED.store(index_enabled && !probe_enabled, Ordering::Release);
    Ok(())
}
