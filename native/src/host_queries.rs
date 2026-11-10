//! Count VirtualQuery imports in three pinned, already-loaded mod DLLs.
//!
//! This is attribution, not a memory-region cache. Every call still reaches the
//! same Windows API with the same arguments and result. The wrappers do only a
//! relaxed atomic increment before the call, so they do not touch last-error or
//! query outputs. No panel callback, game pointer, OS implementation or guard is
//! patched. Missing/different DLLs are explicitly unobserved, not guessed.
//!
//! Modloader loads and permanently retains each DLL before calling its init.
//! In the tested load order AI/ally/multicore precede hf_native; QoL follows it
//! and is deliberately not covered. Installation atomically swaps aligned IAT
//! entries after native hook installation; no game-thread suspension is needed.

use std::{
    ffi::c_void,
    mem::{size_of, zeroed},
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

use windows_sys::Win32::System::{
    LibraryLoader::GetModuleHandleA,
    Memory::{
        VirtualProtect, VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_EXECUTE_READ,
        PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_READONLY, PAGE_READWRITE,
        PAGE_WRITECOPY,
    },
};

struct Module {
    filename: &'static [u8],
    timestamp: u32,
    image_size: u32,
    iat_rva: usize,
    name_rva: usize,
    wrapper: *const (),
}

// SAFETY: The pointer is an immutable wrapper code address; no native state is
// exposed through it. The DLL remains loaded for process lifetime.
unsafe impl Sync for Module {}

// Reviewed directly from the installed PE32+ import tables on 2026-11-10.
// Only these exact identities/layouts are admitted; no generic PE parser or
// dependency update is required for this attribution experiment.
const MODULES: [Module; 3] = [
    Module {
        filename: b"hf_ai_v1.1.0.dll\0",
        timestamp: 0x6af1_cd60,
        image_size: 0x93000,
        iat_rva: 0x7e258,
        name_rva: 0x7e88a,
        wrapper: ai_query as *const (),
    },
    Module {
        filename: b"hf_ally_v1.0.4.dll\0",
        timestamp: 0x6af2_3f7f,
        image_size: 0xde000,
        iat_rva: 0xbf610,
        name_rva: 0xbfd82,
        wrapper: ally_query as *const (),
    },
    Module {
        filename: b"hf_mc_v0.1.0.dll\0",
        timestamp: 0x6af0_8fc3,
        image_size: 0x48000,
        iat_rva: 0x3f0f8,
        name_rva: 0x3f702,
        wrapper: multicore_query as *const (),
    },
];

static CALLS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static ACTIVE: [AtomicBool; 3] = [const { AtomicBool::new(false) }; 3];

#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub calls: [u64; 3],
    pub active: [bool; 3],
}

pub fn snapshot() -> Snapshot {
    Snapshot {
        calls: std::array::from_fn(|i| CALLS[i].load(Ordering::Relaxed)),
        active: std::array::from_fn(|i| ACTIVE[i].load(Ordering::Relaxed)),
    }
}

macro_rules! query_wrapper {
    ($name:ident, $slot:expr) => {
        unsafe extern "system" fn $name(
            address: *const c_void,
            information: *mut MEMORY_BASIC_INFORMATION,
            length: usize,
        ) -> usize {
            CALLS[$slot].fetch_add(1, Ordering::Relaxed);
            // SAFETY: Forward the exact API arguments. Our own IAT is never
            // patched, and installation admits only entries equal to this API.
            unsafe { VirtualQuery(address, information, length) }
        }
    };
}

query_wrapper!(ai_query, 0);
query_wrapper!(ally_query, 1);
query_wrapper!(multicore_query, 2);

fn readable(address: usize, length: usize) -> bool {
    let Some(end) = address.checked_add(length) else {
        return false;
    };
    // SAFETY: MEMORY_BASIC_INFORMATION is a scalar output DTO; VirtualQuery
    // accepts arbitrary probe addresses and never dereferences that address.
    let mut information: MEMORY_BASIC_INFORMATION = unsafe { zeroed() };
    if unsafe {
        VirtualQuery(
            address as *const c_void,
            &mut information,
            size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    } == 0
    {
        return false;
    }
    if information.State != MEM_COMMIT || information.Protect & PAGE_GUARD != 0 {
        return false;
    }
    let protection = information.Protect & 0xff;
    if !matches!(
        protection,
        PAGE_READONLY
            | PAGE_READWRITE
            | PAGE_WRITECOPY
            | PAGE_EXECUTE_READ
            | PAGE_EXECUTE_READWRITE
            | PAGE_EXECUTE_WRITECOPY
    ) {
        return false;
    }
    let start = information.BaseAddress as usize;
    let Some(region_end) = start.checked_add(information.RegionSize) else {
        return false;
    };
    address >= start && end <= region_end
}

/// # Safety
/// Called only for a permanently loaded module obtained from GetModuleHandleA.
/// Each span is independently validated before scalar/byte reads. The pinned
/// NT-header offset is 0x78 in all three admitted DLLs.
unsafe fn validate(module: &Module, base: usize) -> bool {
    if !readable(base, 0x180) {
        return false;
    }
    let u16_at = |offset: usize| unsafe { ((base + offset) as *const u16).read_unaligned() };
    let u32_at = |offset: usize| unsafe { ((base + offset) as *const u32).read_unaligned() };
    if u16_at(0) != 0x5a4d || u32_at(0x3c) != 0x78 || u32_at(0x78) != 0x4550 {
        return false;
    }
    if u16_at(0x7c) != 0x8664 || u16_at(0x90) != 0x20b {
        return false;
    }
    if u32_at(0x80) != module.timestamp || u32_at(0xc8) != module.image_size {
        return false;
    }
    let name = base + module.name_rva;
    if !readable(name, b"VirtualQuery\0".len()) || !readable(base + module.iat_rva, 8) {
        return false;
    }
    // SAFETY: Span checked above; bytes are import-name data, never ownership.
    unsafe { std::slice::from_raw_parts(name as *const u8, 13) == b"VirtualQuery\0" }
}

/// Install optional, atomic import observers after the native patch batch.
/// Failures leave the relevant DLL unobserved, without changing its API calls.
///
/// # Safety
/// Startup only. Modloader permanently retains the target DLLs. This DLL and
/// its own VirtualQuery import must likewise remain resident until process exit.
pub unsafe fn install() {
    for (index, module) in MODULES.iter().enumerate() {
        // SAFETY: Static NUL-terminated filename; do not load absent modules or
        // change their initialization order merely to extend probe coverage.
        let base = unsafe { GetModuleHandleA(module.filename.as_ptr()) } as usize;
        if base == 0 || !unsafe { validate(module, base) } {
            log::warn!(
                "hf_native: host query observer slot {index} unavailable: missing/different DLL"
            );
            continue;
        }
        let address = base + module.iat_rva;
        if !address.is_multiple_of(size_of::<usize>()) {
            log::warn!("hf_native: host query observer slot {index}: unaligned IAT refused");
            continue;
        }
        // SAFETY: Validated aligned, readable process-lifetime IAT pointer.
        let slot = unsafe { &*(address as *const AtomicUsize) };
        let expected = VirtualQuery as *const () as usize;
        if slot.load(Ordering::Acquire) != expected {
            log::warn!("hf_native: host query observer slot {index}: foreign API hook refused");
            continue;
        }

        let mut previous = 0;
        // SAFETY: Protect only the validated live IAT entry, not target code.
        if unsafe { VirtualProtect(address as *const c_void, 8, PAGE_READWRITE, &mut previous) }
            == 0
        {
            log::warn!("hf_native: host query observer slot {index}: IAT protection failed");
            continue;
        }
        let installed = slot
            .compare_exchange(
                expected,
                module.wrapper as usize,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok();
        let mut ignored = 0;
        // SAFETY: Restore the page protection returned by the matching call.
        if unsafe { VirtualProtect(address as *const c_void, 8, previous, &mut ignored) } == 0 {
            log::error!(
                "hf_native: host query observer slot {index}: cannot restore IAT protection"
            );
        }
        ACTIVE[index].store(installed, Ordering::Relaxed);
        log::info!("hf_native: host query observer slot {index} installed={installed}; API/guards unchanged");
    }
}
