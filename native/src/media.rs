//! Observer-only attribution for the native media-name lookup.
//!
//! `CopyNameIntoMedia` (`0x140178950`) selects one of 13 singly linked buckets
//! and compares names byte by byte until the first mismatch. The installed
//! animation bucket contains more than two thousand descriptors. The post-v0.1.3
//! perf capture showed this lookup only in scene-load bursts, but could not say
//! which callers or chain depths caused the samples.
//!
//! This module replaces the seven native CALL sites, not the lookup itself. A
//! bounded shadow scan predicts the result and records its cost, then the
//! untouched native function runs and its result is returned. A completed shadow
//! prediction is compared with that native result. Consequently the probe cannot
//! return a different resource, and any implementation disagreement is visible
//! in `mismatches` instead of becoming a gameplay change.
//!
//! The shadow scan approximately doubles this lookup's work. Its counters are
//! evidence; wall time from a probe-enabled load is deliberately not an
//! optimization measurement. No resource pointer or name is retained. The
//! distinct-name estimate stores only bounded 64-bit hashes, which will decide
//! whether a later cache or an insert-time index is worth designing.

use std::{
    mem::transmute,
    slice,
    sync::atomic::{AtomicU64, Ordering},
};

use patchy::PatchSession;

use crate::call_hook;

const COPY_NAME_INTO_MEDIA: usize = 0x1_4017_8950;
const COPY_NAME_INTO_MEDIA_BODY: [u8; 74] = [
    0x48, 0x63, 0xc2, 0x4c, 0x8b, 0x54, 0xc1, 0x10, 0x4d, 0x85, 0xd2, 0x74, 0x36, 0x0f, 0x1f, 0x00,
    0x4d, 0x8d, 0x4a, 0x08, 0x49, 0x8b, 0xc0, 0x4d, 0x2b, 0xc8, 0x66, 0x0f, 0x1f, 0x44, 0x00, 0x00,
    0x0f, 0xb6, 0x10, 0x42, 0x0f, 0xb6, 0x0c, 0x08, 0x2b, 0xd1, 0x75, 0x07, 0x48, 0xff, 0xc0, 0x85,
    0xc9, 0x75, 0xed, 0x85, 0xd2, 0x74, 0x0f, 0x4d, 0x8b, 0x92, 0x98, 0x00, 0x00, 0x00, 0x4d, 0x85,
    0xd2, 0x75, 0xcd, 0x33, 0xc0, 0xc3, 0x49, 0x8b, 0xc2, 0xc3,
];

const BUCKET_HEADS: usize = 0x10;
const DESCRIPTOR_NAME: usize = 0x08;
const DESCRIPTOR_NEXT: usize = 0x98;

/// The shipped animation bucket has 2249 entries. This cap leaves room for
/// runtime growth while ensuring that a corrupt list cannot make the observer
/// walk forever. Native lookup still runs after a capped shadow scan.
const MAX_SHADOW_HOPS: u64 = 4096;
const MAX_NAME_BYTES: usize = 4096;

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

const ANIMATION: usize = 0;
const TYPE_11: usize = 1;
const SOUND: usize = 2;
const FONT: usize = 3;
const TEXTURE: usize = 4;
const MAP_ANIMATION_A: usize = 5;
const MAP_ANIMATION_B: usize = 6;

const CALL_SITES: [CallSite; 7] = [
    CallSite {
        name: "animation",
        address: 0x1_4017_0241,
        call: [0xe8, 0x0a, 0x87, 0x00, 0x00],
        replacement: animation_lookup as *const (),
    },
    CallSite {
        name: "type11",
        address: 0x1_4017_0281,
        call: [0xe8, 0xca, 0x86, 0x00, 0x00],
        replacement: type_11_lookup as *const (),
    },
    CallSite {
        name: "sound",
        address: 0x1_4017_02ca,
        call: [0xe8, 0x81, 0x86, 0x00, 0x00],
        replacement: sound_lookup as *const (),
    },
    CallSite {
        name: "font",
        address: 0x1_4017_03f1,
        call: [0xe8, 0x5a, 0x85, 0x00, 0x00],
        replacement: font_lookup as *const (),
    },
    CallSite {
        name: "texture",
        address: 0x1_4017_0460,
        call: [0xe8, 0xeb, 0x84, 0x00, 0x00],
        replacement: texture_lookup as *const (),
    },
    CallSite {
        name: "map animation A",
        address: 0x1_4015_d1ff,
        call: [0xe8, 0x4c, 0xb7, 0x01, 0x00],
        replacement: map_animation_a_lookup as *const (),
    },
    CallSite {
        name: "map animation B",
        address: 0x1_4015_d226,
        call: [0xe8, 0x25, 0xb7, 0x01, 0x00],
        replacement: map_animation_b_lookup as *const (),
    },
];

struct Counters {
    calls: AtomicU64,
    hits: AtomicU64,
    misses: AtomicU64,
    hops: AtomicU64,
    compare_bytes: AtomicU64,
    max_hops: AtomicU64,
    capped: AtomicU64,
    mismatches: AtomicU64,
}

impl Counters {
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

static COUNTERS: [Counters; 7] = [const { Counters::new() }; 7];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SiteSnapshot {
    pub calls: u64,
    pub hops: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MediaSnapshot {
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
    pub sites: [SiteSnapshot; 7],
}

/// Reads atomics only; the diagnostic thread never walks native media lists.
pub fn snapshot() -> MediaSnapshot {
    let mut result = MediaSnapshot {
        distinct_names: DISTINCT_NAMES.load(Ordering::Relaxed),
        name_hash_full: NAME_HASH_FULL.load(Ordering::Relaxed),
        name_hash_capped: NAME_HASH_CAPPED.load(Ordering::Relaxed),
        ..MediaSnapshot::default()
    };

    for (site, counters) in COUNTERS.iter().enumerate() {
        let calls = counters.calls.load(Ordering::Relaxed);
        let hops = counters.hops.load(Ordering::Relaxed);
        result.sites[site] = SiteSnapshot { calls, hops };
        result.calls += calls;
        result.hits += counters.hits.load(Ordering::Relaxed);
        result.misses += counters.misses.load(Ordering::Relaxed);
        result.hops += hops;
        result.compare_bytes += counters.compare_bytes.load(Ordering::Relaxed);
        result.max_hops = result
            .max_hops
            .max(counters.max_hops.load(Ordering::Relaxed));
        result.capped += counters.capped.load(Ordering::Relaxed);
        result.mismatches += counters.mismatches.load(Ordering::Relaxed);
    }
    result
}

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
    let mut node = bucket.read_unaligned() as *mut u8;
    let mut hops = 0u64;
    let mut compare_bytes = 0u64;

    while !node.is_null() && hops < MAX_SHADOW_HOPS {
        hops += 1;
        let entry_name = node.add(DESCRIPTOR_NAME);
        let mut offset = 0usize;

        loop {
            let query_byte = name.add(offset).read();
            let entry_byte = entry_name.add(offset).read();
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

        node = (node.add(DESCRIPTOR_NEXT) as *const usize).read_unaligned() as *mut u8;
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
    let mut hash = 0xcbf2_9ce4_8422_2325u64 ^ (resource_type as u32 as u64);
    let mut terminated = false;

    // SAFETY: The native API requires a readable NUL-terminated name. The cap
    // additionally prevents an invalid name from causing unbounded observer work.
    for offset in 0..MAX_NAME_BYTES {
        let byte = unsafe { name.add(offset).read() };
        if byte == 0 {
            terminated = true;
            break;
        }
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
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
unsafe fn run_lookup(media: *mut u8, resource_type: i32, name: *const u8, site: usize) -> *mut u8 {
    let observation = observe_lookup(media, resource_type, name);
    record_name(resource_type, name);

    let lookup: unsafe extern "C" fn(*mut u8, i32, *const u8) -> *mut u8 =
        transmute(COPY_NAME_INTO_MEDIA);
    let native_result = lookup(media, resource_type, name);
    let counters = &COUNTERS[site];

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

macro_rules! lookup_wrapper {
    ($function:ident, $site:expr) => {
        unsafe extern "C" fn $function(
            media: *mut u8,
            resource_type: i32,
            name: *const u8,
        ) -> *mut u8 {
            // SAFETY: Every wrapper preserves the validated native CALL ABI and
            // forwards the original arguments to the untouched native function.
            unsafe { run_lookup(media, resource_type, name, $site) }
        }
    };
}

lookup_wrapper!(animation_lookup, ANIMATION);
lookup_wrapper!(type_11_lookup, TYPE_11);
lookup_wrapper!(sound_lookup, SOUND);
lookup_wrapper!(font_lookup, FONT);
lookup_wrapper!(texture_lookup, TEXTURE);
lookup_wrapper!(map_animation_a_lookup, MAP_ANIMATION_A);
lookup_wrapper!(map_animation_b_lookup, MAP_ANIMATION_B);

/// Validates the native lookup and all seven CALL sites before preparing any
/// patch. No resource cache or replacement result path is installed.
///
/// # Safety
/// Runs once at quiescent Modloader initialization on the supported fixed-base
/// Steam 1.163 image. This DLL and all CALL relays must remain loaded for the
/// process lifetime, and no other module may own these CALL instructions.
pub unsafe fn prepare(session: &mut PatchSession) -> Result<(), String> {
    // SAFETY: The parent establishes the supported executable mapping.
    let native_body = unsafe {
        slice::from_raw_parts(
            COPY_NAME_INTO_MEDIA as *const u8,
            COPY_NAME_INTO_MEDIA_BODY.len(),
        )
    };
    if native_body != COPY_NAME_INTO_MEDIA_BODY {
        return Err("media: CopyNameIntoMedia body differs at 0x140178950".into());
    }

    for site in &CALL_SITES {
        // SAFETY: Same supported-image contract; five bytes cover one native CALL.
        let actual = unsafe { slice::from_raw_parts(site.address as *const u8, 5) };
        if actual != site.call {
            return Err(format!(
                "media: {} CALL differs at {:#x}",
                site.name, site.address
            ));
        }
    }

    for site in &CALL_SITES {
        // SAFETY: Every replacement forwards the exact validated three-argument
        // CALL ABI and preserves the native return address through call_hook.
        unsafe {
            call_hook::prepare_call(session, site.address, &site.call, site.replacement)?;
        }
    }
    Ok(())
}
