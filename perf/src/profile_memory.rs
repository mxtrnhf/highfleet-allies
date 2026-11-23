//! Scalar-only, callback-local observations; never construct owning native layouts.

use std::{ffi::c_void, mem::size_of};
use windows_sys::Win32::System::Memory::{
    VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
    PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_READONLY, PAGE_READWRITE, PAGE_WRITECOPY,
};

/// Read a scalar from a live game-thread observation. The caller must establish
/// native lifetime/threading; page validation alone does not establish either.
pub unsafe fn scalar<T: Copy>(address: usize) -> Option<T> {
    let end = address.checked_add(size_of::<T>())?;
    if address < 0x10000 || size_of::<T>() == 0 {
        return None;
    }
    let mut cursor = address;
    // A scalar occupies at most two regions. Do not retain region validity over
    // an original game call, scene load, or callback boundary.
    for _ in 0..2 {
        let mut region: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: region is writable local storage of the specified size.
        if unsafe {
            VirtualQuery(
                cursor as *const c_void,
                &mut region,
                size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        } == 0
        {
            return None;
        }
        let readable = matches!(
            region.Protect & 0xff,
            PAGE_READONLY
                | PAGE_READWRITE
                | PAGE_WRITECOPY
                | PAGE_EXECUTE_READ
                | PAGE_EXECUTE_READWRITE
                | PAGE_EXECUTE_WRITECOPY
        );
        if region.State != MEM_COMMIT || region.Protect & PAGE_GUARD != 0 || !readable {
            return None;
        }
        let region_end = (region.BaseAddress as usize).checked_add(region.RegionSize)?;
        if region_end <= cursor {
            return None;
        }
        if end <= region_end {
            // SAFETY: callers read only Copy scalars, on the native game thread
            // with live global/object storage. Complete range checked above;
            // unaligned access creates no reference or native ownership.
            return Some(unsafe { (address as *const T).read_unaligned() });
        }
        cursor = region_end;
    }
    None
}

// Native profiler layout verified against Profiler_RecordFrameTime 0x1402ab980.
pub const PROFILERS: [usize; 8] = [
    0x147fc6a88,
    0x143a33c88,
    0x143a33c20,
    0x1439e92b0,
    0x1403c63c0,
    0x1439e6028,
    0x147fc7658,
    0x147fc76b8,
];

// End of the last field read; this bounds pointer arithmetic, not allocation size.
const RECORD_READ_END: usize = 0x390;
const RECORD_RING_INDEX: usize = 0x48;
const RECORD_RING_ENTRIES: i32 = 100;
const RECORD_START_TICK: usize = 0x378;
const RECORD_FREQUENCY: usize = 0x380;
const RECORD_ELAPSED_MICROS: usize = 0x388;

// Box2D world offsets: hf_notes/docs/findings/physics/box2d-struct-layouts.md.
const WORLD_READ_END: usize = 0x19338;
const WORLD_BODY_COUNT: usize = 0x19330;
const WORLD_CONTACT_COUNT: usize = 0x19300;
const WORLD_JOINT_COUNT: usize = 0x19334;
const AI_ENTRY_STRIDE: usize = 0x18;

const BODY_COUNT_LIMIT: u32 = 15_000;
const WORLD_COUNT_LIMIT: u32 = 1_000_000;
const AI_ENTRY_LIMIT: usize = 4096;
const HEURISTIC_SAMPLES: usize = 20;

#[derive(Clone, Copy, Default)]
pub struct Snapshot {
    pointer: usize,
    index: i32,
}

pub unsafe fn snapshot(global: usize) -> Option<Snapshot> {
    // SAFETY: global is a supported image pointer global; its object is observed
    // synchronously on the sole native task thread, without retaining references.
    unsafe {
        let pointer = scalar::<usize>(global)?;
        if pointer < 0x10000 || pointer > isize::MAX as usize - RECORD_READ_END {
            return None;
        }
        let index = scalar::<i32>(pointer + RECORD_RING_INDEX)?;
        (0..RECORD_RING_ENTRIES)
            .contains(&index)
            .then_some(Snapshot { pointer, index })
    }
}

pub unsafe fn fresh_sample(
    global: usize,
    before: Option<Snapshot>,
    start: i64,
    end: i64,
    frequency: i64,
) -> Option<i64> {
    // SAFETY: same synchronous scalar observation contract as snapshot; pages
    // are revalidated after the game call because objects may have been replaced.
    unsafe {
        let before = before?;
        let after = snapshot(global)?;
        // The ring index must advance by exactly one during this frame's call.
        if before.pointer != after.pointer
            || after.index != (before.index + 1) % RECORD_RING_ENTRIES
        {
            return None;
        }
        let clock_start = scalar::<i64>(after.pointer + RECORD_START_TICK)?;
        let clock_frequency = scalar::<i64>(after.pointer + RECORD_FREQUENCY)?;
        if clock_frequency != frequency || clock_start < start || clock_start > end {
            return None;
        }
        let elapsed = scalar::<i64>(after.pointer + RECORD_ELAPSED_MICROS)?;
        let limit = ((end - start) as f64 * 1_000_000.0 / frequency as f64) + 1.0;
        (elapsed >= 0 && elapsed as f64 <= limit).then_some(elapsed)
    }
}

#[derive(Clone, Copy, Default)]
pub struct Workload {
    pub bodies: Option<u32>,
    pub physics_bodies: Option<u32>,
    pub contacts: Option<u32>,
    pub joints: Option<u32>,
    pub ai_entries: Option<usize>,
}

/// Read the game's own 20-entry work-time ring (`DAT_143a1c450` begin,
/// `DAT_143a1c458` end), copy at most 20 floats and sort them. The native
/// heuristic uses the third-fastest sample (`count/8`); returning p25 and p50
/// makes the pacing decision inspectable without writing game state.
///
/// # Safety
///
/// Game-thread scalar observation after the frame; pointers are validated for
/// range and page readability, and no reference is retained.
pub unsafe fn heuristic_ring() -> Option<(f32, f32)> {
    unsafe {
        let begin = scalar::<usize>(0x143a1c450)?;
        let end = scalar::<usize>(0x143a1c458)?;
        if begin < 0x10000 || end < begin {
            return None;
        }
        let bytes = end.checked_sub(begin)?;
        if !bytes.is_multiple_of(4) {
            return None;
        }
        let count = bytes / 4;
        if count == 0 || count > HEURISTIC_SAMPLES {
            return None;
        }
        let mut samples = [0.0f32; HEURISTIC_SAMPLES];
        for (index, slot) in samples.iter_mut().take(count).enumerate() {
            *slot = scalar::<f32>(begin + index * 4)?;
        }
        samples[..count].sort_by(f32::total_cmp);
        Some((samples[count / 8], samples[count / 2]))
    }
}

pub unsafe fn workload() -> Workload {
    // SAFETY: fixed image globals and live Box2D world scalars, read on the game
    // task thread after the frame. No object traversal, virtual calls, references,
    // allocator ownership or retained pointers. Offsets: box2d-struct-layouts.md.
    unsafe {
        let bodies = scalar::<u32>(0x143a51150).filter(|n| *n <= BODY_COUNT_LIMIT);
        let world = scalar::<usize>(0x143a119f0)
            .filter(|p| *p >= 0x10000 && *p <= isize::MAX as usize - WORLD_READ_END);
        let world_count = |offset| {
            world
                .and_then(|p| scalar::<u32>(p + offset))
                .filter(|n| *n <= WORLD_COUNT_LIMIT)
        };
        let ai_entries = scalar::<usize>(0x143faa608)
            .zip(scalar::<usize>(0x143faa610))
            .and_then(|(begin, end)| {
                let bytes = end.checked_sub(begin)?;
                if !bytes.is_multiple_of(AI_ENTRY_STRIDE)
                    || bytes / AI_ENTRY_STRIDE > AI_ENTRY_LIMIT
                {
                    return None;
                }
                if bytes != 0 && (begin < 0x10000 || !begin.is_multiple_of(8)) {
                    return None;
                }
                Some(bytes / AI_ENTRY_STRIDE)
            });
        Workload {
            bodies,
            physics_bodies: world_count(WORLD_BODY_COUNT),
            contacts: world_count(WORLD_CONTACT_COUNT),
            joints: world_count(WORLD_JOINT_COUNT),
            ai_entries,
        }
    }
}
