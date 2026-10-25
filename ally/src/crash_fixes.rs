//! Fixes for crashes in the stock game (1.163). Each patch checks the bytes it replaces and is skipped, with a log
//! line, when they differ (another mod owns the site, or a different build).
//!
//! * `plane_ai`: the aircraft target-choice routine reads a body field (`+0x3c4`) through the brain pointer, past the end
//!   of the 432-byte brain block. Reading the brain's own `+0x48` instead is never 500, which is what the stock code
//!   effectively always answers; the out-of-bounds read is gone (`hf_notes/docs/findings/aircraft-logic-bug-report.md`).
//! * `merge_guard`: scene state 2 of `SomethingWithLevelSerias` passes a null fleet to `0x1400f5b40` when the player-side
//!   vector is empty; the callee dereferences it at once. The guard makes it return 0 ("no ship") for a null argument.
//!   Two more readers of the same vector (`0x1402beb4a`, `0x1402c108d`: element 0 = null, then `mov rcx,[rax+8]`)
//!   are guarded the same way: a null element skips to the code that follows the call, which tests for "no ship".
//!   Not confirmed against a tester's crash, so a possible fix only.

use patchy::{PatchSession, Trampoline};
use patchy::Condition;

const PLANE_SITE: usize = 0x1_4001_0a51;
/// `cmp dword ptr [r14+0x3c4], 0x1f4`
const PLANE_BYTES: [u8; 11] = [0x41, 0x81, 0xbe, 0xc4, 0x03, 0x00, 0x00, 0xf4, 0x01, 0x00, 0x00];

const MERGE_SITE: usize = 0x1_400f_5b4f;
/// `mov rdi, [rcx+0x100]`
const MERGE_BYTES: [u8; 7] = [0x48, 0x8b, 0xb9, 0x00, 0x01, 0x00, 0x00];
/// `xor eax,eax` of the function's "nothing found" exit (its epilogue follows).
const MERGE_EXIT: usize = 0x1_400f_5b82;

/// `mov rcx,[rax+8]; call 0x1400f5b40` after `vector[0]`: (address, bytes).
const ELEMENT_SITES: [(usize, [u8; 9]); 2] = [
    (0x1_402b_eb4a, [0x48, 0x8b, 0x48, 0x08, 0xe8, 0xed, 0x6f, 0xe3, 0xff]),
    (0x1_402c_108d, [0x48, 0x8b, 0x48, 0x08, 0xe8, 0xaa, 0x4a, 0xe3, 0xff]),
];
const MERGE_FN: usize = 0x1_400f_5b40;

unsafe fn bytes_at<'a>(address: usize, len: usize) -> &'a [u8] {
    std::slice::from_raw_parts(address as *const u8, len)
}

pub unsafe fn install(session: &mut PatchSession, plane_ai: bool, merge_guard: bool) {
    if plane_ai {
        let found = bytes_at(PLANE_SITE, PLANE_BYTES.len());
        if found != PLANE_BYTES {
            log::warn!("hf_ally: plane AI fix not applied, bytes at {PLANE_SITE:#x} are {found:02x?}");
        } else {
            let mut patched = PLANE_BYTES;
            patched[3] = 0x48;
            patched[4] = 0x00;
            match session.overwrite(PLANE_SITE, &patched) {
                Ok(_) => log::info!("hf_ally: plane AI crash fix on"),
                Err(e) => log::warn!("hf_ally: plane AI fix failed: {e}"),
            }
        }
    }
    if merge_guard {
        let found = bytes_at(MERGE_SITE, MERGE_BYTES.len());
        if found != MERGE_BYTES {
            log::warn!("hf_ally: fleet merge guard not applied, bytes at {MERGE_SITE:#x} are {found:02x?}");
        } else {
            let mut t = Trampoline::new();
            let ok = t.new_label();
            t.bytes(&[0x48, 0x85, 0xc9]); // test rcx,rcx
            t.jump_if(Condition::NotEqual, ok);
            t.absolute_jump(MERGE_EXIT);
            let _ = t.bind(ok);
            t.bytes(&MERGE_BYTES);
            t.relative_jump(MERGE_SITE + MERGE_BYTES.len());
            match session.detour_trampoline(MERGE_SITE, MERGE_BYTES.len(), t) {
                Ok(_) => log::info!("hf_ally: fleet merge guard on"),
                Err(e) => log::warn!("hf_ally: fleet merge guard failed: {e}"),
            }
        }
        for (site, expected) in ELEMENT_SITES {
            let found = bytes_at(site, expected.len());
            if found != expected {
                log::warn!("hf_ally: fleet vector guard not applied, bytes at {site:#x} are {found:02x?}");
                continue;
            }
            let mut t = Trampoline::new();
            let ok = t.new_label();
            t.bytes(&[0x48, 0x85, 0xc0]); // test rax,rax
            t.jump_if(Condition::NotEqual, ok);
            t.relative_jump(site + expected.len()); // rax = 0: the stock "test rax,rax" after the call sees "no ship"
            let _ = t.bind(ok);
            t.bytes(&[0x48, 0x8b, 0x48, 0x08]); // mov rcx,[rax+8]
            t.absolute_call(MERGE_FN as *const ());
            t.relative_jump(site + expected.len());
            match session.detour_trampoline(site, expected.len(), t) {
                Ok(_) => log::info!("hf_ally: fleet vector guard on at {site:#x}"),
                Err(e) => log::warn!("hf_ally: fleet vector guard at {site:#x} failed: {e}"),
            }
        }
    }
}
