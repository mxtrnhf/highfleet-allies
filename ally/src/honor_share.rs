//! Battle honor sharing: every deployed ally receives a share of the player's in-battle honor.
//!
//! `Creature + 0x1a24` (f32) is the honor/XP earned in the current battle. `battleManagerTick`
//! (0x1402cc260) adds the player's award at 0x1402cc4f7 and stores the new total at 0x1402cc4ff.
//! `SCR_COMBAT_ReturnToMap` (0x1402ce060) later folds each player-fleet ship's `+0x1a24` into its
//! `m_xp` (`+0x1a28`), adds `floor(honor / 500)` to the promotion counter and zeroes the battle
//! field. The team values and the return path are from the 1.163 RE notes; the hook is guarded by
//! an exact byte check before installation.

use std::sync::atomic::{AtomicU32, Ordering};

use patchy::{PatchSession, ReturnType, Trampoline};

use crate::{live_ship, plausible, read, ALLIES, ARMED};

/// `Creature + 0x1a24`: honor earned in this battle; `+0x1a28` is the persistent `m_xp`.
pub const OFF_BATTLE_HONOR: usize = 0x1a24;

/// `battleManagerTick`: `ADDSS XMM1, [RAX+0x1a24]` (the old total plus this award).
const HONOR_ADD: usize = 0x1_402c_c4f7;
const HONOR_ADD_BYTES: [u8; 8] = [0xf3, 0x0f, 0x58, 0x88, 0x24, 0x1a, 0x00, 0x00];
/// The store we replace: `MOVSS [RAX+0x1a24], XMM1` (new total).
const HONOR_STORE: usize = 0x1_402c_c4ff;
const HONOR_STORE_BYTES: [u8; 8] = [0xf3, 0x0f, 0x11, 0x88, 0x24, 0x1a, 0x00, 0x00];
/// The instruction after the store; the trampoline resumes here.
const HONOR_RESUME: usize = 0x1_402c_c507;
const HONOR_RESUME_BYTES: [u8; 8] = [0xf3, 0x0f, 0x10, 0x05, 0xf9, 0xaa, 0xcf, 0x07];

/// Effective `ally_honor_share`, stored as f32 bits (0.5 = 0x3f00_0000).
static SHARE: AtomicU32 = AtomicU32::new(0x3f00_0000);

/// Clamps `ally_honor_share` to 0..=1; non-finite values fall back to 0.5. Returns the value used.
pub fn set_share(share: f32) -> f32 {
    let effective = if share.is_finite() { share.clamp(0.0, 1.0) } else { 0.5 };
    SHARE.store(effective.to_bits(), Ordering::Relaxed);
    effective
}

/// The effective share after `set_share` (default 0.5 until the config is read).
pub fn share() -> f32 {
    f32::from_bits(SHARE.load(Ordering::Relaxed))
}

/// Verifies the exact battle-manager award sequence before the detour is prepared.
pub fn verify_native() -> Result<(), String> {
    for (name, address, expected) in [
        ("honor add", HONOR_ADD, &HONOR_ADD_BYTES[..]),
        ("honor store", HONOR_STORE, &HONOR_STORE_BYTES[..]),
        ("honor resume", HONOR_RESUME, &HONOR_RESUME_BYTES[..]),
    ] {
        let found = unsafe { std::slice::from_raw_parts(address as *const u8, expected.len()) };
        if found != expected {
            return Err(format!("{name} at {address:#x} has unexpected bytes {found:02x?}, expected {expected:02x?}"));
        }
    }
    Ok(())
}

/// Installs the award detour. The callback receives the player creature in RCX and the new total
/// (XMM1 bits) in EDX; `preserved_call` restores the volatile registers before the store is replayed.
///
/// # Safety
/// `verify_native` must have accepted the 1.163 site, and this must run during mod initialization on
/// the game thread before the first battle.
pub unsafe fn install(session: &mut PatchSession) -> Result<(), String> {
    let mut trampoline = Trampoline::new();
    trampoline.preserved_call(
        on_award as *const (),
        &[
            0x48, 0x8b, 0xc8, // mov rcx, rax  ; player creature
            0x66, 0x0f, 0x7e, 0xca, // movd edx, xmm1 ; new total bits
        ],
        &[],
        ReturnType::None,
    );
    trampoline.bytes(&HONOR_STORE_BYTES);
    trampoline.absolute_jump(HONOR_RESUME);
    session.detour_trampoline(HONOR_STORE, HONOR_STORE_BYTES.len(), trampoline).map_err(|e| e.to_string())?;
    Ok(())
}

/// Runs at the player's honor store, before the new total reaches the creature. The native award is
/// left untouched; each live deployed ally gets `earned * share` added to its own battle honor.
unsafe extern "C" fn on_award(player: usize, new_total_bits: u32) {
    let share = f32::from_bits(SHARE.load(Ordering::Relaxed));
    if share <= 0.0 || !ARMED.load(Ordering::Relaxed) || !plausible(player) {
        return;
    }
    let new_total = f32::from_bits(new_total_bits);
    let earned = new_total - read::<f32>(player + OFF_BATTLE_HONOR);
    if !earned.is_finite() || earned <= 0.0 {
        return;
    }
    let Ok(allies) = ALLIES.try_lock() else { return };
    for (index, &ally) in allies.iter().enumerate() {
        // A handoff can leave the current player in the list for one frame; never double-credit.
        if ally == player || allies[..index].contains(&ally) || !live_ship(ally) {
            continue;
        }
        let current = read::<f32>(ally + OFF_BATTLE_HONOR);
        let next = current + earned * share;
        if current.is_finite() && next.is_finite() {
            (ally as *mut f32).byte_add(OFF_BATTLE_HONOR).write_unaligned(next);
        }
    }
}
