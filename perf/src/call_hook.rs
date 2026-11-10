//! Native CALL -> near relay -> tail jump, retaining the native return address.
//!
//! Adapted from native/src/call_hook.rs (27c9220), without the unrelated RBP
//! argument variant. These are independent cdylib crates, not a Cargo workspace.
//! The relay has no frame; the typed C-unwind replacement has normal DLL unwind
//! information. A JMP-entered trampoline containing CALLs cannot provide that.

use std::ffi::c_void;

use patchy::PatchSession;
use windows_sys::Win32::System::{
    Diagnostics::Debug::FlushInstructionCache,
    Memory::{
        VirtualAlloc, VirtualFree, VirtualProtect, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE,
        PAGE_EXECUTE_READ, PAGE_READWRITE,
    },
    Threading::GetCurrentProcess,
};

const ALLOCATION_GRANULARITY: usize = 0x1_0000;
const RELAY_PAGE_BYTES: usize = 0x1000;
const SEARCH_STEPS: usize = 1024;

/// Prepare a guarded five-byte CALL to a nearby absolute-jump relay.
///
/// # Safety
/// Startup-only supported image, no thread executing patch sources. The code at
/// site must be readable and exactly expected. Replacement must match the native
/// Win64 argument/return/unwind ABI. DLL and relay remain resident until exit.
pub unsafe fn prepare_call(
    session: &mut PatchSession,
    site: usize,
    expected: &[u8; 5],
    replacement: *const (),
) -> Result<(), String> {
    // SAFETY: Caller establishes this bounded mapped code range at startup.
    if unsafe { std::slice::from_raw_parts(site as *const u8, 5) } != expected {
        return Err(format!("native CALL differs at {site:#x}"));
    }
    if expected[0] != 0xe8 {
        return Err(format!("expected CALL opcode at {site:#x}"));
    }
    let continuation = site.checked_add(5).ok_or("CALL address overflow")?;
    let relay = allocate_near(site)?;
    let displacement = match i32::try_from(relay as i128 - continuation as i128) {
        Ok(displacement) => displacement,
        Err(_) => {
            // SAFETY: Private reservation, not yet reachable from native code.
            unsafe { VirtualFree(relay, 0, MEM_RELEASE) };
            return Err("relay is outside CALL rel32 range".into());
        }
    };
    let mut code = [0u8; 14];
    code[..6].copy_from_slice(&[0xff, 0x25, 0, 0, 0, 0]);
    code[6..].copy_from_slice(&(replacement as usize).to_le_bytes());
    // SAFETY: Fresh private RW reservation; no published code points here yet.
    unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), relay.cast::<u8>(), code.len()) };
    let mut previous = 0;
    // SAFETY: Private page, writable protection output; no live game storage.
    if unsafe { VirtualProtect(relay, RELAY_PAGE_BYTES, PAGE_EXECUTE_READ, &mut previous) } == 0 {
        let error = std::io::Error::last_os_error();
        unsafe { VirtualFree(relay, 0, MEM_RELEASE) };
        return Err(format!("cannot protect CALL relay: {error}"));
    }
    // SAFETY: Complete RX code span and current-process pseudo handle.
    if unsafe { FlushInstructionCache(GetCurrentProcess(), relay, code.len()) } == 0 {
        let error = std::io::Error::last_os_error();
        unsafe { VirtualFree(relay, 0, MEM_RELEASE) };
        return Err(format!("cannot flush CALL relay: {error}"));
    }
    let mut call = [0xe8, 0, 0, 0, 0];
    call[1..].copy_from_slice(&displacement.to_le_bytes());
    // SAFETY: One complete CALL replaces the guarded CALL, preserving its
    // original return address. Relay tail-jumps without modifying RSP/registers.
    if let Err(error) = unsafe { session.overwrite(site, &call) } {
        // SAFETY: Failed preparation has not published the private relay.
        unsafe { VirtualFree(relay, 0, MEM_RELEASE) };
        return Err(error.to_string());
    }
    // Permanent process-owned RX page. A later batch failure can retain at most
    // three unused pages; init is single-shot. No dangling code is installed.
    Ok(())
}

fn allocate_near(site: usize) -> Result<*mut c_void, String> {
    let origin = site / ALLOCATION_GRANULARITY * ALLOCATION_GRANULARITY;
    for step in 1..=SEARCH_STEPS {
        let distance = step * ALLOCATION_GRANULARITY;
        for candidate in [origin.checked_sub(distance), origin.checked_add(distance)]
            .into_iter()
            .flatten()
        {
            // SAFETY: Reserve fresh private memory, never overwrite mappings.
            let allocation = unsafe {
                VirtualAlloc(
                    candidate as *const c_void,
                    RELAY_PAGE_BYTES,
                    MEM_RESERVE | MEM_COMMIT,
                    PAGE_READWRITE,
                )
            };
            if allocation.is_null() {
                continue;
            }
            if allocation as usize == candidate {
                return Ok(allocation);
            }
            // SAFETY: Only our fresh unpublished reservation is released.
            unsafe { VirtualFree(allocation, 0, MEM_RELEASE) };
        }
    }
    Err("no free CALL-relay page within the bounded 64 MiB search".into())
}
