//! Read-only side of hf_ai's debug-state shared memory (see `highfleet-allies/ai/src/debug_state.rs`).
//!
//! hf_ai publishes one entry per AI ship in `Local\hf_ai_debug_state_v1`; this module opens the
//! mapping lazily and looks a ship up by creature pointer. The layout is duplicated in both DLLs:
//! keep the magic, version, capacity and structs identical, and rename the mapping if the format
//! changes. hf_ally never writes the mapping.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Memory::{MapViewOfFile, OpenFileMappingW, FILE_MAP_READ};

pub const MAP_NAME: &str = "Local\\hf_ai_debug_state_v1";
const MAGIC: u32 = 0x4846_4149; // "HFAI"
const VERSION: u32 = 1;
const CAPACITY: usize = 256;
/// How old an entry may be before the overlay ignores it (the game clock is in seconds).
const STALE_S: f64 = 2.0;
const RETRY_MS: u64 = 2000;

pub const TACTICS_STOCK: u8 = 0;
pub const TACTICS_PANIC: u8 = 1;
pub const TACTICS_DIVE: u8 = 2;
pub const TACTICS_DIVE_SKIP: u8 = 3;
pub const TACTICS_AIM: u8 = 4;

pub const FLARES_NONE: u8 = 0;
pub const FLARES_BURST: u8 = 1;
pub const FLARES_EVADE: u8 = 2;
pub const FLARES_PANIC: u8 = 3;
pub const FLARES_ROLL: u8 = 4;
pub const FLARES_CURTAIN: u8 = 5;

#[repr(C)]
struct Header {
    magic: u32,
    version: u32,
    count: u32,
    reserved: u32,
    generation: u64,
    clock: f64,
}

#[repr(C)]
struct Entry {
    creature: u64,
    clock: f64,
    tactics: u8,
    flares: u8,
    reserved: [u8; 6],
}

#[repr(C)]
struct Region {
    header: Header,
    entries: [Entry; CAPACITY],
}

static VIEW: AtomicUsize = AtomicUsize::new(0);
static LAST_OPEN_MS: AtomicU64 = AtomicU64::new(0);

fn wide_name() -> Vec<u16> {
    MAP_NAME.encode_utf16().chain([0]).collect()
}

unsafe fn view() -> Option<usize> {
    let mapped = VIEW.load(Ordering::Relaxed);
    if mapped != 0 {
        return Some(mapped);
    }
    let now = crate::now_ms();
    if now.wrapping_sub(LAST_OPEN_MS.load(Ordering::Relaxed)) < RETRY_MS {
        return None;
    }
    LAST_OPEN_MS.store(now, Ordering::Relaxed);
    let name = wide_name();
    // SAFETY: the name is NUL-terminated UTF-16; FILE_MAP_READ only asks for read access.
    let handle = unsafe { OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr()) };
    if handle.is_null() {
        return None;
    }
    let size = std::mem::size_of::<Region>();
    // SAFETY: handle is live and the mapping was created by hf_ai with the same size.
    let mapped = unsafe { MapViewOfFile(handle, FILE_MAP_READ, 0, 0, size) }.Value as usize;
    // The view keeps the mapping alive after the handle is closed.
    unsafe { CloseHandle(handle) };
    if mapped == 0 {
        return None;
    }
    VIEW.store(mapped, Ordering::Relaxed);
    Some(mapped)
}

/// State published by hf_ai for one ship. `None` when hf_ai is absent, the mapping is not ready, or the
/// entry is stale.
pub unsafe fn lookup(creature: usize, clock: f64) -> Option<(u8, u8)> {
    let mapped = view()?;
    let header = mapped as *const Header;
    if (*header).magic != MAGIC || (*header).version != VERSION {
        return None;
    }
    let base = (mapped as *const u8).add(std::mem::size_of::<Header>());
    for index in 0..CAPACITY {
        let entry = base.add(index * std::mem::size_of::<Entry>()) as *const Entry;
        if (*entry).creature != creature as u64 {
            continue;
        }
        if (clock - (*entry).clock).abs() > STALE_S {
            return None;
        }
        return Some(((*entry).tactics, (*entry).flares));
    }
    None
}
