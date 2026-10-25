//! Side patch: parts the AI must never treat as a strategic launch to intercept (config `intercept_ignore_oids`).
//!
//! Every AI ship rebuilds its threat list (creature `+0xca0`) each tick in `0x1400625a0`. In the missile-vs-ship
//! arcade scene (flag `0x147fc6fda`) the list takes every body whose stats category has bit `0x8000000` ("strategic
//! launch"), whoever launched it. The first entry becomes the missile target the guns (CIWS) and the anti-air
//! launchers aim at. A part modded into that category (a Sprint) is therefore shot at by the side that launched it.
//! The test is `checkShipPartFlagActive(body, 0x8000000)` at `0x140062812`; this hook answers "no" for a body whose
//! `m_oid` is on the list, so that body is never a candidate and the AI picks the next real threat as it always did.

use std::sync::{
    atomic::{AtomicU32, AtomicUsize, Ordering},
    Mutex, OnceLock,
};

use patchy::{PatchSession, Trampoline};

/// `mov edx,0x8000000; mov rcx,rbx; call checkShipPartFlagActive` (13 bytes); the call is the last 5.
const SITE: usize = 0x1_4006_280a;
const SITE_BYTES: [u8; 13] = [0xba, 0x00, 0x00, 0x00, 0x08, 0x48, 0x8b, 0xcb, 0xe8, 0x59, 0x6c, 0xfe, 0xff];
const CALL_SITE: usize = SITE + 8;
const CHECK_FN: usize = 0x1_4004_9470;

/// Body: the part's `m_oid`, an MSVC `std::string` (inline up to 15 bytes, else a heap pointer in the first 8).
const OFF_OID: usize = 0x190;
const OFF_OID_LEN: usize = 0x1a0;
const OFF_OID_CAP: usize = 0x1a8;
/// No `m_oid` in OL.seria is anywhere near this long; anything longer is not a string we understand.
const MAX_OID: usize = 96;

static IGNORE: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
static LAST_BODY: AtomicUsize = AtomicUsize::new(0);
static LOGGED: AtomicU32 = AtomicU32::new(0);
/// The first distinct `m_oid`s seen on a strategic-launch body, logged once each so a wrong config name can be fixed
/// from one test run.
static SEEN: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

/// The `m_oid` bytes of a body, or `None` when the string looks wrong.
unsafe fn oid_of<'a>(body: usize) -> Option<&'a [u8]> {
    let len = ((body + OFF_OID_LEN) as *const usize).read_unaligned();
    let cap = ((body + OFF_OID_CAP) as *const usize).read_unaligned();
    if len == 0 || len > MAX_OID || cap < len {
        return None;
    }
    let data = if cap <= 15 {
        body + OFF_OID
    } else {
        let ptr = ((body + OFF_OID) as *const usize).read_unaligned();
        if ptr < 0x1_0000 {
            return None;
        }
        ptr
    };
    Some(std::slice::from_raw_parts(data as *const u8, len))
}

/// Called with the body after the stock flag test said "strategic launch": 1 = keep it as a candidate, 0 = ignore.
unsafe extern "C" fn keep(body: usize) -> u8 {
    let _hook = crate::diag::scope("intercept_keep");
    let Some(list) = IGNORE.get() else { return 1 };
    let Some(oid) = oid_of(body) else { return 1 };
    if let Ok(mut seen) = SEEN.try_lock() {
        if seen.len() < 12 && !seen.iter().any(|o| o.as_slice() == oid) {
            seen.push(oid.to_vec());
            log::info!("INTERCEPT candidate: strategic-launch body oid={}", String::from_utf8_lossy(oid));
        }
    }
    if !list.iter().any(|ignored| ignored.as_slice() == oid) {
        return 1;
    }
    // one line per new body, not per ship per tick
    if LAST_BODY.swap(body, Ordering::Relaxed) != body && LOGGED.fetch_add(1, Ordering::Relaxed) < 40 {
        log::info!("INTERCEPT ignore: body={body:#x} oid={} is not a threat to the AI", String::from_utf8_lossy(oid));
    }
    0
}

/// Installs the filter when `oids` is not empty. A non-empty list with a wrong site is an error the caller logs.
pub unsafe fn install(session: &mut PatchSession, oids: &[String]) -> Result<(), String> {
    let list: Vec<Vec<u8>> = oids.iter().map(|s| s.trim().as_bytes().to_vec()).filter(|b| !b.is_empty()).collect();
    if list.is_empty() {
        return Ok(());
    }
    let found = std::slice::from_raw_parts(SITE as *const u8, SITE_BYTES.len());
    if found != SITE_BYTES {
        return Err(format!("threat-list flag test at {SITE:#x} has unexpected bytes {found:02x?}: intercept_ignore_oids not applied"));
    }
    let mut code: Vec<u8> = vec![0x51, 0x48, 0x83, 0xec, 0x28, 0x48, 0xb8]; // push rcx; sub rsp,0x28; mov rax,imm64
    code.extend((CHECK_FN as u64).to_le_bytes());
    code.extend([0xff, 0xd0, 0x48, 0x83, 0xc4, 0x28, 0x59]); // call rax; add rsp,0x28; pop rcx (the body again)
    code.extend([0x84, 0xc0, 0x74, 0x14]); // test al,al; jz +20 (flag not set: nothing to filter)
    code.extend([0x48, 0x83, 0xec, 0x20, 0x48, 0xb8]); // sub rsp,0x20; mov rax,imm64
    code.extend((keep as *const () as usize as u64).to_le_bytes());
    code.extend([0xff, 0xd0, 0x48, 0x83, 0xc4, 0x20]); // call rax; add rsp,0x20 (al = keep)
    let mut trampoline = Trampoline::new();
    trampoline.bytes(&code);
    trampoline.relative_jump(CALL_SITE + 5);
    session.detour_trampoline(CALL_SITE, 5, trampoline).map_err(|e| e.to_string())?;
    log::info!("hf_ally: AI ignores these parts as interception targets: {}", oids.join(", "));
    let _ = IGNORE.set(list);
    Ok(())
}
