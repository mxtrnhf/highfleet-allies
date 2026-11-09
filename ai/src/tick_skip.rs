//! Halves the native AI tick rate of player-side ships (Steam 1.163 only).
//!
//! The stock AI task calls `aiShipDriver` for every AI-list entry every frame
//! (50 Hz). The heavy part inside that tick is the obstacle scan, whose
//! obstacle-list insert is quadratic in the number of visible nearby parts
//! (`AiShip_ScanObstacles` 0x14024cc20, `AiShip_AddObstacleNode` 0x14024ce50;
//! see the 2026-11-09 ai-opt evidence bundle). Skipping the whole tick of
//! every player-side AI ship on alternate frames halves the ally share of
//! that work; enemy ships keep the full tick rate.
//!
//! The skipped calls are the ship's entire tick, so the trade-offs are the
//! native ones: ally commands run 20 ms stale, and the per-tick timers
//! (the 6.0 s aircraft timer `+0xcb8`, the 2.0 s Zenith launch cooldown
//! `+0xd60`, the 0.5 s AA launch cooldown `+0xd64`; see the AiShipTick plate)
//! decrement at half cadence, doubling those launch and re-arm pauses for
//! allies. hf_ai's own tactic layer is game-clock based, so its dodge and
//! dwell timings stay time-correct on both tick rates.
//!
//! The hook wraps the `CALL aiShipDriver` in the `SomethingAITask` loop
//! (0x14001123a). A call-site wrap makes no signature assumptions: at the
//! call `RCX` is the Creature (verified 2026-11-09 in the live 1.163 image),
//! the loop ignores the return value, and the gate leaves every argument
//! register untouched, so the trampoline either performs the original call
//! with the caller's frame intact or jumps straight to the resume code.
//!
//! hf_perf's `ai_spans` wraps the same site. hf_ai loads first (mod load order
//! is alphabetical), so with this lever on hf_perf cannot install its
//! `ai_driver` span and drops it with a log line. Set `ally_tick_skip` false
//! for the attribution capture.

use std::panic::{catch_unwind, AssertUnwindSafe};

use patchy::{Condition, PatchSession, ProcessModule, ReturnType, Trampoline};

use crate::native_layout;
use crate::{plausible, rd};

/// `CALL aiShipDriver` in the `SomethingAITask` loop, plus its resume and
/// target addresses.
const DRIVER_SITE_RVA: usize = 0x11_23a;
const DRIVER_RESUME_RVA: usize = 0x11_23f;
const DRIVER_TARGET_RVA: usize = 0x73_9c0;
/// `CALL rel32` to `aiShipDriver` (0x1400739c0).
const DRIVER_CALL: [u8; 5] = [0xe8, 0x81, 0x27, 0x06, 0x00];

/// `test al, al` right after the gate call: leaves the skip decision in the
/// flags for the conditional jump. The register restore around a callback
/// does not touch the flags.
const TEST_GATE_RESULT: [u8; 2] = [0x84, 0xc0];

/// Offset of a Creature's AI-driver kind selector: 1 = ship (this lever's
/// target), 2 = ground defence (returns at once), 3 = launched missile or
/// aircraft. Read 2026-11-09 in `aiShipDriver`'s prologue
/// (`MOV EAX, [RCX+0xbd8]`, 0x1400739d5); kind semantics per its plate comment.
const CREATURE_KIND: usize = 0xbd8;
const KIND_SHIP: i32 = 1;

/// The frame rate the AI task runs at (fixed 20 ms steps).
const TICKS_PER_SECOND: f64 = 50.0;
/// Global with the game time in seconds, advanced by dt at each frame.
const GAME_TIME: usize = 0x143a119c0;

/// Whether a player-side AI ship's tick should be skipped this frame.
///
/// Never unwinds into the game: an unexpected panic fails open (the tick runs).
unsafe extern "C" fn tick_skip_gate(creature: usize) -> bool {
    match catch_unwind(AssertUnwindSafe(|| gate_decision(creature))) {
        Ok(skip) => skip,
        Err(_) => false,
    }
}

/// The skip rule: a player-side AI-driven ship (kind 1, `side` above zero) is
/// skipped on every other 20 ms frame. Every unreadable field runs the tick.
///
/// # Safety
///
/// `creature` comes from the native AI list and was accepted by the loop's
/// own virtual gate; the reads below are still page-checked by `rd`.
unsafe fn gate_decision(creature: usize) -> bool {
    if !plausible(creature) {
        return false;
    }

    // Launched missiles and aircraft (kind 3) and ground defence (kind 2) are
    // not tick-skipped: their ticks are cheap, and their steer logic is
    // position-based per tick.
    if rd::<i32>(creature + CREATURE_KIND) != Some(KIND_SHIP) {
        return false;
    }

    let Some(side) = rd::<f32>(creature + native_layout::CREATURE_SIDE) else {
        return false;
    };
    // Positive side = the player's side (a wingman, a strike-group ship, or
    // the flagship the player just switched away from); negative = enemy.
    if !(side > 0.0) {
        return false;
    }

    odd_frame()
}

/// Whether this frame is a skip frame, from the frame-quantized game clock.
///
/// The AI task runs once per 20 ms frame and the clock advances by dt at each
/// frame, so the frame index is `now * 50` rounded. A frozen clock (paused
/// frames the AI loop skips anyway) keeps one parity and fails safe: a NaN or
/// negative clock reads as frame 0 through the saturating cast (no skip).
unsafe fn odd_frame() -> bool {
    let Some(now) = rd::<f64>(GAME_TIME) else {
        return false;
    };

    let frame_index = (now * TICKS_PER_SECOND + 0.5) as u64;
    frame_index % 2 == 1
}

/// Prepares the driver call-site detour when the lever is enabled.
///
/// Returns whether the detour was prepared. A code mismatch is logged and
/// skips this lever (the whole mod is unaffected), it is not an error.
///
/// # Safety
///
/// The caller must be in the startup-only installation path with the supported
/// 1.163 image mapped at the fixed base (both checked in `init`).
pub unsafe fn prepare(
    session: &mut PatchSession,
    module: ProcessModule,
    enabled: bool,
) -> Result<bool, String> {
    if !enabled {
        return Ok(false);
    }

    let site = module
        .resolve_rva(DRIVER_SITE_RVA)
        .map_err(|e| e.to_string())?;
    let expected = std::slice::from_raw_parts(site as *const u8, DRIVER_CALL.len());
    if expected != DRIVER_CALL.as_slice() {
        log::error!(
            "hf_ai: ally tick-skip: the aiShipDriver call at {DRIVER_SITE_RVA:#x} has other bytes, leaving it alone"
        );
        return Ok(false);
    }

    let mut trampoline = Trampoline::new();
    let skipped = trampoline.new_label();
    trampoline.preserved_call(
        tick_skip_gate as *const (),
        &[],
        &TEST_GATE_RESULT,
        ReturnType::None,
    );
    trampoline.jump_if(Condition::NotEqual, skipped);
    let driver_target = module
        .resolve_rva(DRIVER_TARGET_RVA)
        .map_err(|e| e.to_string())? as *const ();
    trampoline.absolute_call(driver_target);
    trampoline.bind(skipped).map_err(|e| e.to_string())?;
    trampoline.absolute_jump(
        module
            .resolve_rva(DRIVER_RESUME_RVA)
            .map_err(|e| e.to_string())?,
    );
    session
        .detour_trampoline(site, DRIVER_CALL.len(), trampoline)
        .map(|_| ())
        .map_err(|e| e.to_string())?;

    log::info!(
        "hf_ai: ally tick-skip prepared at {DRIVER_SITE_RVA:#x} (player-side AI ships tick on alternate frames)"
    );
    Ok(true)
}
