//! Replace one native CALL with CALL-to-relay followed by a tail jump.
//!
//! Unlike a JMP/CALL/JMP call-site trampoline, this preserves the original
//! native return address on the stack. A C-unwind replacement can therefore
//! delegate to throwing native allocation without an invented trampoline frame.

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

/// Prepare a native five-byte CALL to a nearby absolute-jump relay.
///
/// # Safety
/// Startup-only, supported fixed-base image; `site` is a readable five-byte CALL
/// with exactly `expected` bytes. `replacement` implements that call's Win64 ABI,
/// return value and unwind contract. The DLL and relay must live until process
/// exit. No native caller may execute while patches are prepared/installed.
pub unsafe fn prepare_call(
    session: &mut PatchSession,
    site: usize,
    expected: &[u8; 5],
    replacement: *const (),
) -> Result<(), String> {
    // SAFETY: Forward the complete caller contract without adding arguments.
    unsafe { prepare_call_inner(session, site, expected, replacement, false) }
}

/// Prepare a CALL whose relay passes the collector's nonvolatile RBP in R8.
///
/// # Safety
/// All requirements of `prepare_call` apply. Additionally, RBP must hold the
/// third callback argument and original R8 must be dead across the native CALL.
pub unsafe fn prepare_call_with_rbp_argument(
    session: &mut PatchSession,
    site: usize,
    expected: &[u8; 5],
    replacement: *const (),
) -> Result<(), String> {
    // SAFETY: The caller establishes the additional collector register contract.
    unsafe { prepare_call_inner(session, site, expected, replacement, true) }
}

unsafe fn prepare_call_inner(
    session: &mut PatchSession,
    site: usize,
    expected: &[u8; 5],
    replacement: *const (),
    pass_rbp: bool,
) -> Result<(), String> {
    // SAFETY: The supported image/startup contract makes this code span live.
    if unsafe { std::slice::from_raw_parts(site as *const u8, 5) } != expected {
        return Err(format!("native CALL differs at {site:#x}"));
    }
    if expected[0] != 0xe8 {
        return Err(format!("expected CALL opcode at {site:#x}"));
    }
    let continuation = site.checked_add(5).ok_or("CALL address overflow")?;
    let relay = allocate_near(site)?;
    let distance = relay as i128 - continuation as i128;
    let displacement = match i32::try_from(distance) {
        Ok(displacement) => displacement,
        Err(_) => {
            // SAFETY: This private reservation has not been exposed to native code.
            unsafe { VirtualFree(relay, 0, MEM_RELEASE) };
            return Err("relay is outside CALL rel32 range".into());
        }
    };
    let mut storage = [0u8; 17];
    let prefix_length = if pass_rbp {
        storage[..3].copy_from_slice(&[0x4c, 0x8b, 0xc5]); // MOV R8,RBP
        3
    } else {
        0
    };
    let code = &mut storage[..prefix_length + 14];
    code[prefix_length..prefix_length + 6].copy_from_slice(&[0xff, 0x25, 0, 0, 0, 0]);
    code[prefix_length + 6..].copy_from_slice(&(replacement as usize).to_le_bytes());
    // SAFETY: Fresh private RW storage, not executable or reachable yet.
    unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), relay.cast::<u8>(), code.len()) };
    let mut old_protection = 0;
    // SAFETY: Private page reservation remains live; output is writable stack storage.
    if unsafe {
        VirtualProtect(
            relay,
            RELAY_PAGE_BYTES,
            PAGE_EXECUTE_READ,
            &mut old_protection,
        )
    } == 0
    {
        let error = std::io::Error::last_os_error();
        // SAFETY: No patch points at this private page yet.
        unsafe { VirtualFree(relay, 0, MEM_RELEASE) };
        return Err(format!("cannot protect CALL relay: {error}"));
    }
    // SAFETY: The private code span is complete and RX; current-process pseudo handle.
    if unsafe { FlushInstructionCache(GetCurrentProcess(), relay, code.len()) } == 0 {
        let error = std::io::Error::last_os_error();
        // SAFETY: No patch points at this private page yet.
        unsafe { VirtualFree(relay, 0, MEM_RELEASE) };
        return Err(format!("cannot flush CALL relay: {error}"));
    }
    let mut call = [0xe8, 0, 0, 0, 0];
    call[1..].copy_from_slice(&displacement.to_le_bytes());
    // SAFETY: One complete CALL replaces one validated complete CALL; the relay
    // tail-jumps to an ABI-compatible callee with the original return address.
    if let Err(error) = unsafe { session.overwrite(site, &call) } {
        // SAFETY: An unsuccessful prepare did not publish this relay.
        unsafe { VirtualFree(relay, 0, MEM_RELEASE) };
        return Err(error.to_string());
    }
    // Keep the RX relay for process lifetime, like permanent patchy code pages.
    // If a later prepare/install fails, at most five unused pages remain owned
    // by the process. Initialization is single-shot; no dangling jump is created.
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
            // SAFETY: Reserve fresh private address space, never overwrite an
            // existing mapping. VirtualAlloc fails if the requested span is busy.
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
            // SAFETY: Only our just-created reservation is released.
            unsafe { VirtualFree(allocation, 0, MEM_RELEASE) };
        }
    }
    Err("no free CALL-relay page within the bounded 64 MiB search".into())
}
