//! Observer for scene-load `RecursiveFindByID` walks.
//!
//! The post-v0.1.3 profile showed this recursive pre-order search only during
//! scene reconstruction. A query-result memo is the wrong shape for that traffic:
//! 250 of the 269 sampled register IDs were distinct. A build-once ID index could
//! answer first-time queries, but live Ghidra review found two blockers to shipping
//! one now: IDs are re-randomized during the rebuild, and escadra relinking
//! alternates between the escadra and a global scene root. Some virtual relink/load
//! implementations also still lack a complete child/ID mutation audit.
//!
//! This module therefore installs observation, not an index. Two CALL relays
//! bracket `Scene_RebuildFromProfile`; three external query CALL relays run a
//! bounded shadow DFS and then call the untouched native `RecursiveFindByID`.
//! The native recursive CALL is not patched. Native always supplies the return
//! value, and completed shadow walks are checked against it.
//!
//! The first query for each root in a pass censuses the whole subtree instead of
//! stopping at a match. That measures the one-time index construction cost. Later
//! queries stop at the first pre-order match and measure native search work. At
//! most 32 roots are tracked per pass, 1024 frames are held on the shadow stack,
//! and 1 000 000 node visits are allowed per observation. Every cap makes the
//! observation incomplete; it never changes the native call or its result.
//!
//! Like the existing link registry hooks, these paths are native-game-thread
//! code. The diagnostic thread reads only atomics. Root addresses are cleared at
//! pass end and are never exposed to the diagnostic thread or used as a cache.

use std::{
    cell::UnsafeCell,
    mem::transmute,
    slice,
    sync::atomic::{AtomicU64, Ordering},
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

const MAX_ROOTS_PER_PASS: usize = 32;
const MAX_SHADOW_DEPTH: usize = 1024;
const MAX_SHADOW_VISITS: u64 = 1_000_000;

#[derive(Clone, Copy)]
struct CallSite {
    name: &'static str,
    address: usize,
    call: [u8; 5],
    replacement: *const (),
}

// The pointer is an immutable replacement-function address used by the relay.
unsafe impl Sync for CallSite {}

const PASS_LOAD: usize = 0;
const PASS_LEVEL: usize = 1;
const PASS_CALL_SITES: [CallSite; 2] = [
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

struct QueryCounters {
    calls: AtomicU64,
    visits: AtomicU64,
}

impl QueryCounters {
    const fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
            visits: AtomicU64::new(0),
        }
    }
}

static QUERY_COUNTERS: [QueryCounters; 3] = [const { QueryCounters::new() }; 3];
static PASSES_STARTED: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static PASSES_COMPLETED: AtomicU64 = AtomicU64::new(0);
static NESTED_PASSES: AtomicU64 = AtomicU64::new(0);
static PASS_TOTAL_US: AtomicU64 = AtomicU64::new(0);
static PASS_MAX_US: AtomicU64 = AtomicU64::new(0);
static QUERIES_OUTSIDE_PASS: AtomicU64 = AtomicU64::new(0);
static MAX_QUERY_VISITS: AtomicU64 = AtomicU64::new(0);
static DISTINCT_ROOTS: AtomicU64 = AtomicU64::new(0);
static ROOT_SWITCHES: AtomicU64 = AtomicU64::new(0);
static ROOT_TABLE_FULL: AtomicU64 = AtomicU64::new(0);
static CENSUS_NODES: AtomicU64 = AtomicU64::new(0);
static MAX_CENSUS_NODES: AtomicU64 = AtomicU64::new(0);
static SHADOW_INCOMPLETE: AtomicU64 = AtomicU64::new(0);
static SHADOW_MISMATCHES: AtomicU64 = AtomicU64::new(0);

struct PassState {
    depth: u32,
    previous_root: usize,
    roots: [usize; MAX_ROOTS_PER_PASS],
}

struct PassCell(UnsafeCell<PassState>);

// SAFETY: Pass and query relays execute on the native game thread, which also
// owns the native trees being searched. The diagnostic thread never accesses
// this cell. No root address survives the end of its outermost pass.
unsafe impl Sync for PassCell {}

static PASS: PassCell = PassCell(UnsafeCell::new(PassState {
    depth: 0,
    previous_root: 0,
    roots: [0; MAX_ROOTS_PER_PASS],
}));

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QuerySnapshot {
    pub calls: u64,
    pub visits: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RelinkSnapshot {
    pub passes_load: u64,
    pub passes_level: u64,
    pub passes_completed: u64,
    pub nested_passes: u64,
    pub pass_total_us: u64,
    pub pass_max_us: u64,
    pub queries: u64,
    pub queries_outside_pass: u64,
    pub visits: u64,
    pub max_query_visits: u64,
    pub distinct_roots: u64,
    pub root_switches: u64,
    pub root_table_full: u64,
    pub census_nodes: u64,
    pub max_census_nodes: u64,
    pub shadow_incomplete: u64,
    pub shadow_mismatches: u64,
    pub sites: [QuerySnapshot; 3],
}

/// Reads atomics only; in particular this never inspects `PASS` or a Node.
pub fn snapshot() -> RelinkSnapshot {
    let mut result = RelinkSnapshot {
        passes_load: PASSES_STARTED[PASS_LOAD].load(Ordering::Relaxed),
        passes_level: PASSES_STARTED[PASS_LEVEL].load(Ordering::Relaxed),
        passes_completed: PASSES_COMPLETED.load(Ordering::Relaxed),
        nested_passes: NESTED_PASSES.load(Ordering::Relaxed),
        pass_total_us: PASS_TOTAL_US.load(Ordering::Relaxed),
        pass_max_us: PASS_MAX_US.load(Ordering::Relaxed),
        queries_outside_pass: QUERIES_OUTSIDE_PASS.load(Ordering::Relaxed),
        max_query_visits: MAX_QUERY_VISITS.load(Ordering::Relaxed),
        distinct_roots: DISTINCT_ROOTS.load(Ordering::Relaxed),
        root_switches: ROOT_SWITCHES.load(Ordering::Relaxed),
        root_table_full: ROOT_TABLE_FULL.load(Ordering::Relaxed),
        census_nodes: CENSUS_NODES.load(Ordering::Relaxed),
        max_census_nodes: MAX_CENSUS_NODES.load(Ordering::Relaxed),
        shadow_incomplete: SHADOW_INCOMPLETE.load(Ordering::Relaxed),
        shadow_mismatches: SHADOW_MISMATCHES.load(Ordering::Relaxed),
        ..RelinkSnapshot::default()
    };

    for (site, counters) in QUERY_COUNTERS.iter().enumerate() {
        let calls = counters.calls.load(Ordering::Relaxed);
        let visits = counters.visits.load(Ordering::Relaxed);
        result.sites[site] = QuerySnapshot { calls, visits };
        result.queries += calls;
        result.visits += visits;
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

// SAFETY: Shadow walks are non-reentrant native-game-thread operations. The
// original query is called only after its shadow walk has finished, and native
// recursion is not hooked. Diagnostics never access this storage.
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
/// `root` must be the live native Node supplied to the query. Native child list
/// invariants are the same ones required by `RecursiveFindByID`; caps turn a
/// malformed or oversized walk into an incomplete observation, never a result.
unsafe fn observe_find(root: *mut u8, id: u64, census: bool) -> Observation {
    // SAFETY: The caller establishes non-reentrant game-thread execution. Only
    // frames below `depth` are read, and each is initialized before it is pushed.
    let stack = unsafe { &mut *SHADOW_STACK.0.get() };
    let mut depth = 1usize;
    stack[0] = Frame::new(root);
    let mut predicted = std::ptr::null_mut();
    let mut visits = 0u64;

    while depth != 0 {
        if visits >= MAX_SHADOW_VISITS {
            return Observation {
                predicted,
                visits,
                complete: false,
            };
        }

        let frame = &mut stack[depth - 1];
        if !frame.entered {
            frame.entered = true;
            visits += 1;

            if (frame.node.add(NODE_ID) as *const u64).read_unaligned() == id && predicted.is_null()
            {
                predicted = frame.node;
                if !census {
                    return Observation {
                        predicted,
                        visits,
                        complete: true,
                    };
                }
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
                predicted,
                visits,
                complete: false,
            };
        }
        stack[depth] = Frame::new(child);
        depth += 1;
    }

    Observation {
        predicted,
        visits,
        complete: true,
    }
}

/// Returns whether this is the first query for `root` in the current pass.
///
/// # Safety
/// Game-thread only; called with a live native root while the pass bracket is
/// active. Only this function mutates the small root bookkeeping in `PASS`.
unsafe fn register_root(root: *mut u8) -> bool {
    let state = unsafe { &mut *PASS.0.get() };
    if state.depth == 0 {
        return false;
    }

    if state.previous_root != 0 && state.previous_root != root as usize {
        ROOT_SWITCHES.fetch_add(1, Ordering::Relaxed);
    }
    state.previous_root = root as usize;

    if state.roots.contains(&(root as usize)) {
        return false;
    }
    if let Some(slot) = state.roots.iter_mut().find(|slot| **slot == 0) {
        *slot = root as usize;
        DISTINCT_ROOTS.fetch_add(1, Ordering::Relaxed);
        return true;
    }

    ROOT_TABLE_FULL.fetch_add(1, Ordering::Relaxed);
    false
}

/// # Safety
/// Forwards the native query ABI. Native remains authoritative even when the
/// shadow observation is incomplete or disagrees with its result.
unsafe fn run_query(root: *mut u8, id: u64, site: usize) -> *mut u8 {
    // SAFETY: All query relays are game-thread calls into the same native walk.
    let inside_pass = unsafe { (*PASS.0.get()).depth != 0 };
    let census = unsafe { register_root(root) };
    let observation = unsafe { observe_find(root, id, census) };

    let native: unsafe extern "C" fn(*mut u8, u64) -> *mut u8 = transmute(RECURSIVE_FIND_BY_ID);
    let native_result = unsafe { native(root, id) };

    let counters = &QUERY_COUNTERS[site];
    counters.calls.fetch_add(1, Ordering::Relaxed);
    counters
        .visits
        .fetch_add(observation.visits, Ordering::Relaxed);
    MAX_QUERY_VISITS.fetch_max(observation.visits, Ordering::Relaxed);

    if !inside_pass {
        QUERIES_OUTSIDE_PASS.fetch_add(1, Ordering::Relaxed);
    }
    if census && observation.complete {
        CENSUS_NODES.fetch_add(observation.visits, Ordering::Relaxed);
        MAX_CENSUS_NODES.fetch_max(observation.visits, Ordering::Relaxed);
    }
    if !observation.complete {
        SHADOW_INCOMPLETE.fetch_add(1, Ordering::Relaxed);
    } else if observation.predicted != native_result {
        SHADOW_MISMATCHES.fetch_add(1, Ordering::Relaxed);
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
/// Called by the pass wrapper on the game thread before entering native rebuild.
unsafe fn pass_enter(site: usize) {
    PASSES_STARTED[site].fetch_add(1, Ordering::Relaxed);
    let state = unsafe { &mut *PASS.0.get() };
    if state.depth == 0 {
        state.previous_root = 0;
        state.roots = [0; MAX_ROOTS_PER_PASS];
    } else {
        NESTED_PASSES.fetch_add(1, Ordering::Relaxed);
    }
    state.depth += 1;
}

/// # Safety
/// Balances one `pass_enter` on the same native thread, including C-unwind exit.
unsafe fn pass_leave() {
    let state = unsafe { &mut *PASS.0.get() };
    if state.depth == 0 {
        return;
    }
    state.depth -= 1;
    if state.depth == 0 {
        state.previous_root = 0;
        state.roots = [0; MAX_ROOTS_PER_PASS];
    }
}

struct PassGuard;

impl Drop for PassGuard {
    fn drop(&mut self) {
        // SAFETY: The guard is created immediately after pass_enter and dropped
        // on the same thread on both normal and C-unwind exits.
        unsafe { pass_leave() };
    }
}

/// # Safety
/// Calls the native rebuild by its verified absolute address and returns its
/// full RAX result unchanged. The guard only clears observer state.
unsafe fn run_rebuild(scene: *mut u8, site: usize) -> u64 {
    unsafe { pass_enter(site) };
    let guard = PassGuard;
    let started = Instant::now();

    let native: unsafe extern "C-unwind" fn(*mut u8) -> u64 = transmute(SCENE_REBUILD_FROM_PROFILE);
    let result = unsafe { native(scene) };

    let elapsed_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    PASSES_COMPLETED.fetch_add(1, Ordering::Relaxed);
    PASS_TOTAL_US.fetch_add(elapsed_us, Ordering::Relaxed);
    PASS_MAX_US.fetch_max(elapsed_us, Ordering::Relaxed);
    drop(guard);
    result
}

unsafe extern "C-unwind" fn load_rebuild(scene: *mut u8) -> u64 {
    // SAFETY: Validated CALL replacement for the native rebuild signature.
    unsafe { run_rebuild(scene, PASS_LOAD) }
}

unsafe extern "C-unwind" fn level_rebuild(scene: *mut u8) -> u64 {
    // SAFETY: Validated CALL replacement for the native rebuild signature.
    unsafe { run_rebuild(scene, PASS_LEVEL) }
}

/// Validates both pass brackets, all three external queries, the native query
/// body and the rebuild prologue before preparing any of the five CALL relays.
/// No native query instruction is replaced, and no ID index is installed.
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

    for site in PASS_CALL_SITES.iter().chain(QUERY_CALL_SITES.iter()) {
        // SAFETY: Five bytes cover the complete native CALL being validated.
        let actual = unsafe { slice::from_raw_parts(site.address as *const u8, 5) };
        if actual != site.call {
            return Err(format!(
                "relink: {} CALL differs at {:#x}",
                site.name, site.address
            ));
        }
    }

    for site in PASS_CALL_SITES.iter().chain(QUERY_CALL_SITES.iter()) {
        // SAFETY: Each replacement implements its validated native CALL ABI and
        // delegates to the original absolute target for the authoritative result.
        unsafe {
            call_hook::prepare_call(session, site.address, &site.call, site.replacement)?;
        }
    }
    Ok(())
}
