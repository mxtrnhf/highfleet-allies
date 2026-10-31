//! Tactical AI carrier launch groups, Steam 1.163. Evidence and test recipe:
//! hf_notes/docs/findings/ai/tactical-carrier-launch-groups.md.
//!
//! Keep the stock deck checks, plane selection, arming and state transition. Mark a
//! successful arming, then replace only that attempt's six-second cooldown store.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Mutex, OnceLock};

use patchy::{PatchSession, ProcessModule, ReturnType, Trampoline};

use crate::plane_group_math::{Config, Group, STOCK_COOLDOWN};

const ARMED_RVA: usize = 0x65bde;
// mov rcx,[rdi+0xa20], immediately after the AI's call to loadAmmoForAircraft.
const ARMED_BYTES: [u8; 7] = [0x48, 0x8b, 0x8f, 0x20, 0x0a, 0x00, 0x00];
const ARM_CALL_RVA: usize = 0x65bd9;
const ARM_CALL_BYTES: [u8; 5] = [0xe8, 0x52, 0x8d, 0xfc, 0xff];
const COOLDOWN_RVA: usize = 0x65c07;
// mov dword ptr [r14+0xcb8],6.0f. Replaced in full, not replayed.
const COOLDOWN_BYTES: [u8; 11] = [
    0x41, 0xc7, 0x86, 0xb8, 0x0c, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x40,
];
const AIRCRAFT_SELECTOR: usize = 0x1403bd008;
const SCRIPTED_SCENARIO: usize = 0x147fc6fdb;
const FRAME_DELTA: usize = 0x1403c126c;
const MAX_CARRIERS: usize = 256;

static CONFIG: OnceLock<Config> = OnceLock::new();
static GROUPS: Mutex<Option<HashMap<Key, Entry>>> = Mutex::new(None);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    carrier: usize,
    node_id: u32,
}

#[derive(Default)]
struct Entry {
    group: Group,
    last_seen: f64,
}

/// Only SCR_COMBAT's aircraft selector and a non-scripted scene are eligible.
fn context(carrier: usize) -> Option<(Key, f64)> {
    // SAFETY: each game-memory access is page-checked.
    unsafe {
        if crate::rd::<i32>(AIRCRAFT_SELECTOR) != Some(4)
            || crate::rd::<u8>(SCRIPTED_SCENARIO) != Some(0)
            || crate::rd::<u8>(carrier + 0xbd4)? == 0
            || crate::rd::<i32>(carrier + 0x48)? == 0
        {
            return None;
        }
        let now = crate::rd::<f64>(crate::CLOCK).filter(|t| t.is_finite())?;
        Some((
            Key {
                carrier,
                node_id: crate::rd(carrier + 0x18)?,
            },
            now,
        ))
    }
}

unsafe extern "C" fn on_armed(carrier: usize, plane: usize) {
    let _ = catch_unwind(AssertUnwindSafe(|| armed(carrier, plane)));
}

fn armed(carrier: usize, plane: usize) {
    let Some(config) = CONFIG.get().copied() else {
        return;
    };
    let Some((key, now)) = context(carrier) else {
        return;
    };
    // SAFETY: the stock call just armed this selected plane; do not count a failed allocation.
    if unsafe { crate::rd::<usize>(plane + 0xa20) }.unwrap_or(0) == 0 {
        return;
    }
    let mut guard = GROUPS.lock().unwrap_or_else(|e| e.into_inner());
    let groups = guard.get_or_insert_with(HashMap::new);
    groups.retain(|_, entry| now >= entry.last_seen && now - entry.last_seen <= f64::from(config.delay) + 1.0);
    if !groups.contains_key(&key) && groups.len() >= MAX_CARRIERS {
        return;
    }
    let entry = groups.entry(key).or_default();
    entry.group.launched(now, config);
    entry.last_seen = now;
}

unsafe extern "C" fn on_cooldown(carrier: usize) -> f32 {
    catch_unwind(AssertUnwindSafe(|| cooldown(carrier))).unwrap_or(STOCK_COOLDOWN)
}

fn cooldown(carrier: usize) -> f32 {
    let context = context(carrier);
    let mut guard = GROUPS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(groups) = guard.as_mut() else {
        return STOCK_COOLDOWN;
    };
    let Some((key, now)) = context else {
        groups.retain(|key, _| key.carrier != carrier);
        return STOCK_COOLDOWN;
    };
    // SAFETY: the stock tick subtracts this same delta at 0x1400658b1.
    let delta =
        unsafe { crate::rd::<f32>(FRAME_DELTA) }.filter(|delta| delta.is_finite() && *delta >= 0.0);
    let Some(delta) = delta else {
        groups.remove(&key);
        return STOCK_COOLDOWN;
    };
    let Some(entry) = groups.get_mut(&key) else {
        return STOCK_COOLDOWN;
    };
    // A marker is valid only in the same tick as the arming. Failed deck checks or
    // an empty hangar take the stock cooldown and cancel the unfinished group.
    if entry.last_seen != now {
        groups.remove(&key);
        return STOCK_COOLDOWN;
    }
    entry.group.finish_attempt(delta)
}

fn trampoline(callback: *const (), args: &[u8], result: &[u8]) -> Trampoline {
    let mut code = Trampoline::new();
    // Save flags and keep the AI tick's 16-byte call-site stack alignment.
    code.bytes(&[0x9c, 0x48, 0x83, 0xec, 0x08]);
    code.preserved_call(callback, args, result, ReturnType::None);
    code.bytes(&[0x48, 0x83, 0xc4, 0x08, 0x9d]);
    code
}

/// Checks every site before adding either detour; the caller installs them in one session.
pub unsafe fn prepare(
    session: &mut PatchSession,
    module: ProcessModule,
    size: i32,
    delay: f32,
) -> Result<(), String> {
    let config = Config::sanitized(size, delay);
    if config.size == 1 {
        log::info!("hf_ai: plane groups disabled (one plane per stock launch)");
        return Ok(());
    }
    for (rva, expected) in [
        (ARM_CALL_RVA, ARM_CALL_BYTES.as_slice()),
        (ARMED_RVA, ARMED_BYTES.as_slice()),
        (COOLDOWN_RVA, COOLDOWN_BYTES.as_slice()),
    ] {
        let address = module.resolve_rva(rva).map_err(|e| e.to_string())?;
        if crate::rd::<u8>(address).is_none()
            || std::slice::from_raw_parts(address as *const u8, expected.len()) != expected
        {
            log::error!("hf_ai: plane group site {rva:#x} differs, keeping stock launches");
            return Ok(());
        }
    }
    let _ = CONFIG.set(config);
    prepare_armed(session, module)?;
    prepare_cooldown(session, module)?;
    log::info!(
        "hf_ai: tactical AI plane groups: {} planes, {:.2} s apart",
        config.size,
        config.delay
    );
    Ok(())
}

unsafe fn prepare_armed(session: &mut PatchSession, module: ProcessModule) -> Result<(), String> {
    // mov rcx,r14 (carrier); mov rdx,rdi (selected plane).
    let mut code = trampoline(
        on_armed as *const (),
        &[0x4c, 0x89, 0xf1, 0x48, 0x89, 0xfa],
        &[],
    );
    code.bytes(&ARMED_BYTES);
    code.absolute_jump(
        module
            .resolve_rva(ARMED_RVA + ARMED_BYTES.len())
            .map_err(|e| e.to_string())?,
    );
    session
        .detour_trampoline(
            module.resolve_rva(ARMED_RVA).map_err(|e| e.to_string())?,
            ARMED_BYTES.len(),
            code,
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}

unsafe fn prepare_cooldown(
    session: &mut PatchSession,
    module: ProcessModule,
) -> Result<(), String> {
    // Store the callback's f32 before preserved_call restores the original XMM0.
    // movss [r14+0xcb8],xmm0; original integer registers and flags are also restored.
    let store = [0xf3, 0x41, 0x0f, 0x11, 0x86, 0xb8, 0x0c, 0x00, 0x00];
    let mut code = trampoline(on_cooldown as *const (), &[0x4c, 0x89, 0xf1], &store);
    code.absolute_jump(
        module
            .resolve_rva(COOLDOWN_RVA + COOLDOWN_BYTES.len())
            .map_err(|e| e.to_string())?,
    );
    session
        .detour_trampoline(
            module
                .resolve_rva(COOLDOWN_RVA)
                .map_err(|e| e.to_string())?,
            COOLDOWN_BYTES.len(),
            code,
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}
