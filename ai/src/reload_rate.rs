//! Reload countdown multiplier in Part_Tick, Steam 1.163.
//! See hf_notes/docs/findings/combat/guns-aim-reload-fire.md, section 4.

use std::sync::atomic::{AtomicU32, Ordering};

use patchy::{PatchSession, ProcessModule, ReturnType, Trampoline};

const SITE_RVA: usize = 0x554bb;
// mulss xmm0,xmm7; movss xmm1,[rbx+0x864]; subss xmm1,xmm0;
// movss [rbx+0x864],xmm1
const SITE_BYTES: [u8; 24] = [
    0xf3, 0x0f, 0x59, 0xc7, 0xf3, 0x0f, 0x10, 0x8b, 0x64, 0x08, 0x00, 0x00, 0xf3, 0x0f, 0x5c, 0xc8,
    0xf3, 0x0f, 0x11, 0x8b, 0x64, 0x08, 0x00, 0x00,
];
const FRAME_DELTA: usize = 0x1403c126c;
const RELOAD_TIMER: usize = 0x864;
const DEFAULT_RATE: f32 = 1.75;
static ALLY_RATE: AtomicU32 = AtomicU32::new(DEFAULT_RATE.to_bits());
static ENEMY_RATE: AtomicU32 = AtomicU32::new(DEFAULT_RATE.to_bits());

fn sanitized(value: f32) -> f32 {
    if value.is_finite() && (0.0..=100.0).contains(&value) {
        value
    } else {
        log::warn!("hf_ai: invalid reload multiplier {value}; using {DEFAULT_RATE}");
        DEFAULT_RATE
    }
}

/// Called only after the native reload gates and bounds checks have passed.
unsafe extern "C" fn countdown(part: usize, owner: usize, non_player: u32) -> f32 {
    // SAFETY: Part_Tick owns this live part and has just loaded its timer and
    // owner on the game thread. Only scalar fields are read; no references or
    // ownership of game objects are created. A missing/non-creature owner keeps
    // the native non-player multiplier, as do unknown side values.
    let rate = if non_player == 0 {
        DEFAULT_RATE
    } else if owner != 0
        && crate::rd::<u8>(owner + crate::OFF_CODE).is_some_and(|code| code & 0x2f == 0x2f)
    {
        match crate::rd::<f32>(owner + crate::OFF_SIDE) {
            Some(side) if side > 0.0 => f32::from_bits(ALLY_RATE.load(Ordering::Relaxed)),
            Some(side) if side < 0.0 => f32::from_bits(ENEMY_RATE.load(Ordering::Relaxed)),
            _ => 1.0,
        }
    } else {
        1.0
    };
    let timer = (part as *const u8)
        .add(RELOAD_TIMER)
        .cast::<f32>()
        .read_unaligned();
    let delta = (FRAME_DELTA as *const f32).read_unaligned();
    timer - delta * rate
}

pub unsafe fn prepare(
    session: &mut PatchSession,
    module: ProcessModule,
    ally: f32,
    enemy: f32,
) -> Result<(), String> {
    let address = module.resolve_rva(SITE_RVA).map_err(|e| e.to_string())?;
    if crate::rd::<u8>(address).is_none()
        || std::slice::from_raw_parts(address as *const u8, SITE_BYTES.len()) != SITE_BYTES
    {
        log::error!("hf_ai: reload site differs; keeping native reload rates");
        return Ok(());
    }
    let ally = sanitized(ally);
    let enemy = sanitized(enemy);
    ALLY_RATE.store(ally.to_bits(), Ordering::Relaxed);
    ENEMY_RATE.store(enemy.to_bits(), Ordering::Relaxed);

    let mut code = Trampoline::new();
    // Preserve flags and restore the call-site's 16-byte stack alignment.
    code.bytes(&[0x9c, 0x48, 0x83, 0xec, 0x08]);
    // rcx = part (rbx), rdx = owner (rsi), r8d = non-player (r14b).
    let args = [0x48, 0x89, 0xd9, 0x48, 0x89, 0xf2, 0x45, 0x0f, 0xb6, 0xc6];
    // Store the callback's result before preserved_call restores xmm0.
    let result = [0xf3, 0x0f, 0x11, 0x83, 0x64, 0x08, 0x00, 0x00];
    code.preserved_call(countdown as *const (), &args, &result, ReturnType::None);
    code.bytes(&[0x48, 0x83, 0xc4, 0x08, 0x9d]);
    // The next native comparison consumes xmm1.
    code.bytes(&[0xf3, 0x0f, 0x10, 0x8b, 0x64, 0x08, 0x00, 0x00]);
    code.absolute_jump(
        module
            .resolve_rva(SITE_RVA + SITE_BYTES.len())
            .map_err(|e| e.to_string())?,
    );
    session
        .detour_trampoline(address, SITE_BYTES.len(), code)
        .map_err(|e| e.to_string())?;
    log::info!("hf_ai: reload multipliers: allies {ally}, enemies {enemy}");
    Ok(())
}
