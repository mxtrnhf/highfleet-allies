//! Guards native ship-missile launches for AI-run allies (config `ally_zenith_autofire`).
//!
//! `FUN_14002ff60(launcher)` is the ship Zenith launch entry. Both the stock AI tick
//! (`0x140065edb`) and the player's launch key call it, and `hf_ai`'s burst wrapper wraps
//! the same AI call site. Guarding the function entry therefore covers the stock AI, an
//! ally burst and the player path without touching `hf_ai`'s patched bytes. RE evidence:
//! `hf_notes/docs/findings/combat/combat-missile-launch.md`.
//!
//! A blocked launch returns `AL = 0`, the same as a native refusal, and has no side
//! effects. Enemy, neutral and player-flown launches still run the original function.
//! Ship bombers also suppress launches while a bomber course owns the ship's steering.

use std::mem::offset_of;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use patchy::{PatchSession, ReturnType, Trampoline};

use crate::diag::rd;
use crate::native_layout::{Creature, Link, Node};
use crate::{plausible, tracked_ally};

/// `mov [rsp+0x10],rbx` (5 bytes, whole instruction).
const MISSILE_LAUNCH: usize = 0x1_4002_ff60;
const MISSILE_BYTES: [u8; 5] = [0x48, 0x89, 0x5c, 0x24, 0x10];
/// The launching part's owner (the ship) at `Node::owner` -> `Link::target`.
const OFF_OWNER: usize = offset_of!(Node, owner) + offset_of!(Link, target);
/// Native AI control flag: 0 = the ship the player flies, non-zero = AI-run.
const OFF_AI_FLAG: usize = offset_of!(Creature, ai_flag);
/// Shortest gap between "launch held" log lines, in game seconds.
const LOG_INTERVAL_S: f64 = 10.0;

/// True = AI-run allies may launch their own ship missiles (default, vanilla behaviour).
/// False = an ally controlled by the native AI holds its racks; the player can still fire
/// manually from the ship they fly.
static AUTOFIRE: AtomicBool = AtomicBool::new(true);
/// Game-clock bits of the last held-launch log.
static LAST_BLOCK_LOG: AtomicU64 = AtomicU64::new(0);
static BLOCK_LOGGED: AtomicBool = AtomicBool::new(false);

/// Set before the hook is installed.
pub fn set_autofire(allowed: bool) {
    AUTOFIRE.store(allowed, Ordering::Relaxed);
}

/// Logs the first held launch and then at most one line per [`LOG_INTERVAL_S`], so the AI's
/// roughly once-per-second retries cannot flood the log.
unsafe fn log_blocked(ship: usize) {
    let Some(now) = rd::<f64>(crate::CLOCK) else { return };
    let first = !BLOCK_LOGGED.swap(true, Ordering::Relaxed);
    let last = f64::from_bits(LAST_BLOCK_LOG.load(Ordering::Relaxed));
    if !first && (!now.is_finite() || now - last < LOG_INTERVAL_S) {
        return;
    }
    LAST_BLOCK_LOG.store(now.to_bits(), Ordering::Relaxed);
    log::info!("ALLY zenith: c={ship:#x} AI launch held (ally_zenith_autofire off)");
}

/// 1 = hold the launch, 0 = run the original launch function.
unsafe extern "C" fn block_launch(launcher: usize) -> u8 {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(ship) = rd::<usize>(launcher + OFF_OWNER).filter(|ship| plausible(*ship)) else {
            return 0;
        };
        if !tracked_ally(ship) {
            return 0;
        }
        // ai_flag 0 is the ship the player flies: its own launch key keeps working.
        if !AUTOFIRE.load(Ordering::Relaxed) && rd::<u8>(ship + OFF_AI_FLAG) != Some(0) {
            log_blocked(ship);
            return 1;
        }
        if crate::bomber::blocks_launch(ship) {
            return 1;
        }
        0
    }))
    .unwrap_or(1)
}

/// Install only on the supported executable; the caller owns/commits the patch session.
/// # Safety
/// Must run during mod initialization, before AI callbacks, on Steam 1.163's fixed image.
pub unsafe fn install(session: &mut PatchSession) -> Result<(), String> {
    let found = std::slice::from_raw_parts(MISSILE_LAUNCH as *const u8, MISSILE_BYTES.len());
    if found != &MISSILE_BYTES[..] {
        return Err(format!(
            "missile launch at {MISSILE_LAUNCH:#x} has unexpected bytes {found:02x?}"
        ));
    }
    let mut missile = Trampoline::new();
    // Function-entry RSP is 8 mod 16; preserved_call expects an aligned inline site.
    missile.bytes(&[0x48, 0x83, 0xec, 0x08]);
    missile.preserved_call(block_launch as *const (), &[], &[], ReturnType::Rax);
    missile.bytes(&[0x48, 0x83, 0xc4, 0x08]);
    // test al,al; jz replay; xor eax,eax; ret (the native launch reports failure).
    missile.bytes(&[0x84, 0xc0, 0x74, 0x03, 0x31, 0xc0, 0xc3]);
    missile.bytes(&MISSILE_BYTES);
    missile.absolute_jump(MISSILE_LAUNCH + MISSILE_BYTES.len());
    session
        .detour_trampoline(MISSILE_LAUNCH, MISSILE_BYTES.len(), missile)
        .map_err(|error| error.to_string())?;
    log::info!(
        "hf_ally: zenith launch guard installed (AI autofire {})",
        if AUTOFIRE.load(Ordering::Relaxed) { "on" } else { "off" }
    );
    Ok(())
}
