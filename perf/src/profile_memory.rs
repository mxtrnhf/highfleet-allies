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
        if pointer < 0x10000 || pointer > isize::MAX as usize - 0x390 {
            return None;
        }
        let index = scalar::<i32>(pointer + 0x48)?;
        (0..100)
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
        if before.pointer != after.pointer || after.index != (before.index + 1) % 100 {
            return None;
        }
        let clock_start = scalar::<i64>(after.pointer + 0x378)?;
        let clock_frequency = scalar::<i64>(after.pointer + 0x380)?;
        if clock_frequency != frequency || clock_start < start || clock_start > end {
            return None;
        }
        let elapsed = scalar::<i64>(after.pointer + 0x388)?;
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
        if count == 0 || count > 20 {
            return None;
        }
        let mut samples = [0.0f32; 20];
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
        let bodies = scalar::<u32>(0x143a51150).filter(|n| *n <= 15_000);
        let world = scalar::<usize>(0x143a119f0)
            .filter(|p| *p >= 0x10000 && *p <= isize::MAX as usize - 0x19338);
        let world_count = |offset| {
            world
                .and_then(|p| scalar::<u32>(p + offset))
                .filter(|n| *n <= 1_000_000)
        };
        let ai_entries = scalar::<usize>(0x143faa608)
            .zip(scalar::<usize>(0x143faa610))
            .and_then(|(begin, end)| {
                let bytes = end.checked_sub(begin)?;
                if !bytes.is_multiple_of(0x18) || bytes / 0x18 > 4096 {
                    return None;
                }
                if bytes != 0 && (begin < 0x10000 || !begin.is_multiple_of(8)) {
                    return None;
                }
                Some(bytes / 0x18)
            });
        Workload {
            bodies,
            physics_bodies: world_count(0x19330),
            contacts: world_count(0x19300),
            joints: world_count(0x19334),
            ai_entries,
        }
    }
}
