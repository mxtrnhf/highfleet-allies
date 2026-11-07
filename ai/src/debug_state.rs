//! Publishes the per-ship hf_ai state to a named shared-memory mapping for hf_ally's debug overlay.
//!
//! hf_ai and hf_ally are separate DLLs, so this is the only small cross-mod channel: a fixed,
//! versioned, read-only-for-the-reader table in `Local\hf_ai_debug_state_v1`. hf_ai creates the
//! mapping at init and updates one entry per AI ship; hf_ally opens it read-only. Both sides write
//! and read it on the game thread, so no cross-thread lock is needed. The layout is duplicated in
//! `highfleet-allies/ally/src/debug_state.rs`; change both together and bump the mapping name.
//!
//! The two state bytes are independent:
//! * `tactics` is the last `tactics_math::State` decision for the ship;
//! * `flares` is the active `ai_flares` mode (burst / evade / panic / roll); value 5 is reserved for
//!   the retired curtain mode so the shared layout stays compatible with older hf_ally builds.
//! hf_ally shows the flare mode first, because it overrides tactics while it is active.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, FILE_MAP_ALL_ACCESS, PAGE_READWRITE,
};

pub const MAP_NAME: &str = "Local\\hf_ai_debug_state_v1";
const MAGIC: u32 = 0x4846_4149; // "HFAI"
const VERSION: u32 = 1;
const CAPACITY: usize = 256;

/// `tactics_math::State` codes. Kept as plain bytes so the shared layout has no Rust enum ABI.
pub const TACTICS_STOCK: u8 = 0;
pub const TACTICS_PANIC: u8 = 1;
pub const TACTICS_DIVE: u8 = 2;
pub const TACTICS_DIVE_SKIP: u8 = 3;
pub const TACTICS_AIM: u8 = 4;

/// `ai_flares` modes, in priority order for display.
pub const FLARES_NONE: u8 = 0;
pub const FLARES_BURST: u8 = 1;
pub const FLARES_EVADE: u8 = 2;
pub const FLARES_PANIC: u8 = 3;
pub const FLARES_ROLL: u8 = 4;
/// Retired curtain mode; the value stays reserved for the shared hf_ally ABI.
#[expect(dead_code, reason = "value 5 is reserved for the shared hf_ally debug ABI")]
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
    /// Creature pointer; 0 = unused.
    creature: u64,
    /// Game clock of the last update.
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
static INIT_TRIED: AtomicBool = AtomicBool::new(false);

fn wide_name() -> Vec<u16> {
    MAP_NAME.encode_utf16().chain([0]).collect()
}

/// Creates the mapping once at init. A failure disables the channel, not the mod.
pub fn init() {
    if INIT_TRIED.swap(true, Ordering::Relaxed) {
        return;
    }
    let size = std::mem::size_of::<Region>();
    let name = wide_name();
    // SAFETY: the mapping name is a NUL-terminated UTF-16 string; the size covers the whole Region.
    let handle = unsafe {
        CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            std::ptr::null(),
            PAGE_READWRITE,
            0,
            size as u32,
            name.as_ptr(),
        )
    };
    if handle.is_null() {
        log::warn!("hf_ai: cannot create the debug-state mapping; the hf_ally overlay will only show the stock AI state");
        return;
    }
    // SAFETY: handle is a live mapping handle; the view is the same size the mapping was created with.
    let view = unsafe { MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, size) }.Value as usize;
    if view == 0 {
        log::warn!("hf_ai: cannot map the debug-state region; the hf_ally overlay will only show the stock AI state");
        return;
    }
    // SAFETY: the view is mapped for size_of::<Region>() bytes and starts zeroed for a new mapping.
    unsafe {
        let header = view as *mut Header;
        if (*header).magic != MAGIC || (*header).version != VERSION {
            std::ptr::write_bytes(view as *mut u8, 0, size);
            (*header).magic = MAGIC;
            (*header).version = VERSION;
        }
    }
    VIEW.store(view, Ordering::Relaxed);
    log::info!("hf_ai: debug-state mapping ready for hf_ally's debug overlay");
}

unsafe fn region() -> Option<*mut Region> {
    let view = VIEW.load(Ordering::Relaxed);
    (view != 0).then_some(view as *mut Region)
}

unsafe fn entry_at(index: usize) -> *mut Entry {
    let base = (VIEW.load(Ordering::Relaxed) as *mut u8).add(std::mem::size_of::<Header>());
    base.add(index * std::mem::size_of::<Entry>()) as *mut Entry
}

unsafe fn entry_for(creature: usize) -> Option<*mut Entry> {
    let mut free = None;
    let mut oldest = None;
    let mut oldest_clock = f64::INFINITY;
    for index in 0..CAPACITY {
        let entry = entry_at(index);
        let owner = (*entry).creature;
        if owner == creature as u64 {
            return Some(entry);
        }
        if owner == 0 {
            free = free.or(Some(entry));
            continue;
        }
        let clock = (*entry).clock;
        if clock < oldest_clock {
            oldest_clock = clock;
            oldest = Some(entry);
        }
    }
    let entry = free.or(oldest)?;
    (*entry).creature = creature as u64;
    (*entry).clock = 0.0;
    (*entry).tactics = TACTICS_STOCK;
    (*entry).flares = FLARES_NONE;
    (*entry).reserved = [0; 6];
    Some(entry)
}

unsafe fn touch(creature: usize, clock: f64, update: impl FnOnce(*mut Entry)) {
    if region().is_none() {
        return;
    }
    let Some(entry) = entry_for(creature) else { return };
    update(entry);
    (*entry).clock = clock;
    let header = VIEW.load(Ordering::Relaxed) as *mut Header;
    (*header).generation = (*header).generation.wrapping_add(1);
    (*header).clock = clock;
}

/// Stores the last tactics decision for `creature`.
pub fn set_tactics(creature: usize, clock: f64, state: u8) {
    // SAFETY: the mapping pointer, when present, points at the Region created in init().
    unsafe { touch(creature, clock, |entry| (*entry).tactics = state) }
}

/// Stores the active ai_flares mode for `creature`.
pub fn set_flares(creature: usize, clock: f64, mode: u8) {
    // SAFETY: the mapping pointer, when present, points at the Region created in init().
    unsafe { touch(creature, clock, |entry| (*entry).flares = mode) }
}

/// Drops every entry when a new battle is detected; stale clocks would also hide them, but this is immediate.
pub fn clear() {
    let Some(region) = (unsafe { region() }) else { return };
    // SAFETY: the whole Region is mapped and writable.
    unsafe {
        let base = (region as *mut u8).add(std::mem::size_of::<Header>());
        std::ptr::write_bytes(base, 0, CAPACITY * std::mem::size_of::<Entry>());
        let header = region as *mut Header;
        (*header).generation = (*header).generation.wrapping_add(1);
        (*header).count = 0;
    }
}
