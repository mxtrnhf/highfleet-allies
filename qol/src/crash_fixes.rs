//! Stock Steam 1.163 crash fixes, moved from hf_ally without changing patch sites.
//!
//! Aircraft target choice reads Body +0x3c4 through a 432-byte brain pointer.
//! Read its own +0x48 instead, preserving the stock comparison's effective result.
//! Evidence: hf_notes/docs/findings/aircraft-logic-bug-report.md.
//!
//! The fleet guards return "no ship" for a null fleet and skip two null vector
//! elements before dereferencing them. These are defensive guards, not a confirmed
//! explanation or complete fix for every fleet-merge crash.

use patchy::{Condition, PatchSession, Result, Trampoline};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleA;

const IMAGE_BASE: usize = 0x1_4000_0000;
const PLANE_SITE: usize = 0x1_4001_0a51;
// cmp dword ptr [r14+0x3c4],0x1f4
const PLANE_BYTES: [u8; 11] = [
    0x41, 0x81, 0xbe, 0xc4, 0x03, 0x00, 0x00, 0xf4, 0x01, 0x00, 0x00,
];
const MERGE_SITE: usize = 0x1_400f_5b4f;
// mov rdi,[rcx+0x100]
const MERGE_BYTES: [u8; 7] = [0x48, 0x8b, 0xb9, 0x00, 0x01, 0x00, 0x00];
// xor eax,eax followed by the existing "nothing found" epilogue.
const MERGE_EXIT: usize = 0x1_400f_5b82;
const MERGE_FN: usize = 0x1_400f_5b40;
// mov rcx,[rax+8]; call MERGE_FN, after reading vector[0].
const ELEMENT_SITES: [(usize, [u8; 9]); 2] = [
    (
        0x1_402b_eb4a,
        [0x48, 0x8b, 0x48, 0x08, 0xe8, 0xed, 0x6f, 0xe3, 0xff],
    ),
    (
        0x1_402c_108d,
        [0x48, 0x8b, 0x48, 0x08, 0xe8, 0xaa, 0x4a, 0xe3, 0xff],
    ),
];

unsafe fn bytes_at<const N: usize>(address: usize) -> [u8; N] {
    // SAFETY: only fixed instruction sites in the verified Steam 1.163 image are
    // read during initialization. Copy bytes, not game-owned objects or references.
    (address as *const [u8; N]).read_unaligned()
}

/// Prepares enabled crash fixes, skipping and logging sites with changed bytes.
/// Patch preparation errors propagate so the caller does not install a partial session.
///
/// # Safety
/// Caller must verify Steam 1.163 and synchronize patch installation with the game.
pub unsafe fn prepare(session: &mut PatchSession, plane_ai: bool, merge_guard: bool) -> Result {
    if !plane_ai && !merge_guard {
        return Ok(());
    }
    if GetModuleHandleA(std::ptr::null()) as usize != IMAGE_BASE {
        log::warn!("qol: stock crash fixes skipped: unexpected executable image base");
        return Ok(());
    }
    if plane_ai {
        prepare_plane_ai(session)?;
    }
    if merge_guard {
        prepare_merge_guards(session)?;
    }
    Ok(())
}

unsafe fn prepare_plane_ai(session: &mut PatchSession) -> Result {
    let mut patched = PLANE_BYTES;
    patched[3] = 0x48;
    patched[4] = 0x00;
    let found = bytes_at::<11>(PLANE_SITE);
    if found == patched {
        // Old ally versions may already own this identical overwrite.
        log::info!("qol: plane AI crash fix already present, leaving it alone");
        return Ok(());
    }
    if found != PLANE_BYTES {
        log::warn!("qol: plane AI fix skipped: bytes at {PLANE_SITE:#x} are {found:02x?}");
        return Ok(());
    }
    session.overwrite(PLANE_SITE, &patched)?;
    log::info!("qol: plane AI crash fix prepared");
    Ok(())
}

unsafe fn prepare_merge_guards(session: &mut PatchSession) -> Result {
    let found = bytes_at::<7>(MERGE_SITE);
    if found != MERGE_BYTES {
        // Never chain or overwrite an unknown detour, including one from an old ally DLL.
        log::warn!("qol: null-fleet guard skipped: bytes at {MERGE_SITE:#x} are {found:02x?}");
    } else {
        let mut trampoline = Trampoline::new();
        let non_null = trampoline.new_label();
        trampoline.bytes(&[0x48, 0x85, 0xc9]); // test rcx,rcx
        trampoline.jump_if(Condition::NotEqual, non_null);
        trampoline.absolute_jump(MERGE_EXIT);
        trampoline.bind(non_null)?;
        trampoline.bytes(&MERGE_BYTES);
        trampoline.relative_jump(MERGE_SITE + MERGE_BYTES.len());
        session.detour_trampoline(MERGE_SITE, MERGE_BYTES.len(), trampoline)?;
        log::info!("qol: null-fleet crash guard prepared");
    }
    for (site, expected) in ELEMENT_SITES {
        let found = bytes_at::<9>(site);
        if found != expected {
            log::warn!("qol: fleet-vector guard skipped: bytes at {site:#x} are {found:02x?}");
            continue;
        }
        let mut trampoline = Trampoline::new();
        let non_null = trampoline.new_label();
        trampoline.bytes(&[0x48, 0x85, 0xc0]); // test rax,rax
        trampoline.jump_if(Condition::NotEqual, non_null);
        // RAX remains zero: the native test after the call sees "no ship".
        trampoline.relative_jump(site + expected.len());
        trampoline.bind(non_null)?;
        trampoline.bytes(&[0x48, 0x8b, 0x48, 0x08]); // mov rcx,[rax+8]
        trampoline.absolute_call(MERGE_FN as *const ());
        trampoline.relative_jump(site + expected.len());
        session.detour_trampoline(site, expected.len(), trampoline)?;
        log::info!("qol: fleet-vector crash guard prepared at {site:#x}");
    }
    Ok(())
}
