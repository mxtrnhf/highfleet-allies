//! Lets AI ships empty their Zenith racks in a quick burst (Steam 1.163 only).
//!
//! Moved here from the qol fork (`ai_missile_dump_*` in `qol.json`, commit 95e8bc7); behaviour is identical. Turn the qol
//! option off (`ai_missile_dump_chance` 0) when this one is on: if both patch the sites, the second sees changed bytes
//! and leaves them alone.
//!
//! In the AI ship tick `FUN_140062f90`, a ship that has a target in range tries to launch with a
//! roughly 1% chance per tick. It then loops over its launcher list and calls `FUN_14002ff60`
//! on each launcher until one launches. After that launch the loop stops and the ship's launch
//! cooldown (`+0xd60`, 2 seconds) starts, so ships fire one missile every few seconds:
//!
//! ```text
//! 140065c6c  CALL    [RAX+0x110]     ; the roll: random(xmm1, xmm2) in XMM0, RCX = the engine
//! 140065edb  CALL    0x14002ff60     ; RCX = the launcher, AL = 1 if it launched
//! 140065ee2  JNZ     0x140065ef2     ; launched: start the cooldown and stop
//! ```
//!
//! After a launch of an enemy ship the launch is reported as "not launched" with the configured
//! chance, so the cooldown does not start, and the ship is put in a burst. A ship in a burst
//! waits a short random time, then its roll is made to pass so that the game's own loop launches
//! the next missile. Every launch rolls again, so a burst continues until a roll fails, and then
//! the normal cooldown starts. A burst that never gets to launch (the target is gone or out of
//! range) times out.

use std::arch::naked_asm;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard};

use patchy::{PatchSession, Result, ReturnType};

use crate::native_layout;
use crate::{plausible, rd, rng};

/// Address of the `CALL [RAX+0x110]` that rolls the dice for a launch attempt.
const ROLL_ADDRESS: usize = 0x140065c6c;
/// Address of the `CALL FUN_14002ff60` in the loop over the ship's launchers.
const LAUNCH_ADDRESS: usize = 0x140065edb;

/// `CALL qword ptr [RAX+0x110]`.
const ROLL_EXPECTED: [u8; 6] = [0xff, 0x90, 0x10, 0x01, 0x00, 0x00];
/// `CALL rel32` to [`LAUNCH`].
const LAUNCH_EXPECTED: [u8; 5] = [0xe8, 0x80, 0xa0, 0xfc, 0xff];
/// The launch function that is wrapped.
const LAUNCH: usize = 0x14002ff60;
/// Offset in a launcher of the ship it belongs to.
const OWNER_OFFSET: usize = native_layout::NODE_OWNER;
/// Offset in a ship of its side: negative and non-zero for the AI's enemies.
const SIDE_OFFSET: usize = native_layout::CREATURE_SIDE;
/// Offset in a ship of its AI control flag (0 = player-flown).
const AI_FLAG_OFFSET: usize = native_layout::CREATURE_AI_FLAG;
/// Offset in the engine object's function table of the random number function.
const RANDOM_SLOT: usize = 0x110;
/// Global with the game time in seconds.
const GAME_TIME: usize = 0x143a119c0;

/// Default shortest and longest wait between the launches of a burst, in seconds.
pub const DEFAULT_MIN_DELAY: f32 = 0.6;
pub const DEFAULT_MAX_DELAY: f32 = 1.5;
/// Longest wait that can be configured, in seconds.
const MAX_CONFIGURABLE_DELAY: f32 = 10.0;
/// How long a burst waits for its next launch before it is dropped, in seconds.
const BURST_TIMEOUT: f64 = 4.0;

/// The chance, as the bits of an `f32`, written once at init before the hooks can run.
static DUMP_CHANCE: AtomicU32 = AtomicU32::new(0);
/// The shortest and longest wait between launches, as the bits of an `f32`, written once at init.
static MIN_DELAY: AtomicU32 = AtomicU32::new(0);
static MAX_DELAY: AtomicU32 = AtomicU32::new(0);
static INCLUDE_ALLIES: AtomicBool = AtomicBool::new(false);

fn delay_range() -> (f64, f64) {
    (
        f64::from(f32::from_bits(MIN_DELAY.load(Ordering::Relaxed))),
        f64::from(f32::from_bits(MAX_DELAY.load(Ordering::Relaxed))),
    )
}

/// A ship that is in the middle of a burst.
struct Burst {
    /// The ship.
    ship: usize,
    /// Game time from which the next launch may happen.
    next: f64,
    /// Game time after which the burst is dropped.
    expires: f64,
}

static BURSTS: Mutex<Vec<Burst>> = Mutex::new(Vec::new());

fn bursts() -> MutexGuard<'static, Vec<Burst>> {
    BURSTS.lock().unwrap_or_else(|error| error.into_inner())
}

unsafe fn game_time() -> Option<f64> {
    rd::<f64>(GAME_TIME).filter(|t| t.is_finite())
}

fn eligible_ship(ship: usize) -> bool {
    if !plausible(ship) {
        return false;
    }
    let Some(side) = (unsafe { rd::<f32>(ship + SIDE_OFFSET) }) else {
        return false;
    };
    if side < 0.0 {
        return true;
    }
    INCLUDE_ALLIES.load(Ordering::Relaxed)
        && side > 0.0
        && unsafe { rd::<u8>(ship + AI_FLAG_OFFSET) }.is_some_and(|flag| flag != 0)
}

/// Drops the bursts that ran out, or that belong to an earlier battle (the game time restarted).
fn drop_stale(bursts: &mut Vec<Burst>, now: f64) {
    let longest = delay_range().1 + BURST_TIMEOUT + 1.0;
    bursts.retain(|burst| burst.expires >= now && burst.expires - now <= longest);
}

/// Prepares the patches if the chance is above zero.
pub unsafe fn patch_ai_missile_dump(
    session: &mut PatchSession,
    chance: f32,
    min_delay: f32,
    max_delay: f32,
    allies: bool,
) -> Result {
    if !(chance > 0.0) {
        return Ok(());
    }

    let roll = std::slice::from_raw_parts(ROLL_ADDRESS as *const u8, ROLL_EXPECTED.len());
    let launch = std::slice::from_raw_parts(LAUNCH_ADDRESS as *const u8, LAUNCH_EXPECTED.len());
    if roll != ROLL_EXPECTED || launch != LAUNCH_EXPECTED {
        log::error!("hf_ai: missile dump: the game code is not what was expected, leaving it alone");
        return Ok(());
    }

    rng::seed();
    INCLUDE_ALLIES.store(allies, Ordering::Relaxed);
    DUMP_CHANCE.store(chance.to_bits(), Ordering::Relaxed);
    // Not finite or negative: the default. The maximum is never below the minimum.
    let min_delay = if min_delay.is_finite() && min_delay >= 0.0 {
        min_delay.min(MAX_CONFIGURABLE_DELAY)
    } else {
        DEFAULT_MIN_DELAY
    };
    let max_delay = if max_delay.is_finite() && max_delay >= 0.0 {
        max_delay.min(MAX_CONFIGURABLE_DELAY)
    } else {
        DEFAULT_MAX_DELAY
    }
    .max(min_delay);
    MIN_DELAY.store(min_delay.to_bits(), Ordering::Relaxed);
    MAX_DELAY.store(max_delay.to_bits(), Ordering::Relaxed);
    session.patch_call(
        ROLL_ADDRESS,
        roll_trampoline as *const (),
        ROLL_EXPECTED.len(),
        false,
        ReturnType::Xmm0,
    )?;
    session.patch_call(
        LAUNCH_ADDRESS,
        launch_hook as *const (),
        LAUNCH_EXPECTED.len(),
        false,
        ReturnType::Rax,
    )?;

    log::info!(
        "hf_ai: AI missile dump patches prepared at {ROLL_ADDRESS:#x} and {LAUNCH_ADDRESS:#x} (chance: {chance:.2}, delay {min_delay:.2}..{max_delay:.2} s)"
    );
    Ok(())
}

/// Adds the ship (`R14` in the AI tick) as the fourth argument, in `R9`.
#[unsafe(naked)]
unsafe extern "C" fn roll_trampoline() {
    naked_asm!(
        "mov r9, r14",
        "jmp {helper}",
        helper = sym roll_hook,
    );
}

/// The launch roll: random in `[low, high]`. A ship in a burst gets `high` (a launch) once its
/// wait is over and `low` (no launch) before that.
unsafe extern "C" fn roll_hook(engine: *mut u8, low: f32, high: f32, ship: usize) -> f32 {
    let override_roll = catch_unwind(AssertUnwindSafe(|| {
        let mut bursts = bursts();
        if !bursts.is_empty() {
            if let Some(now) = game_time() {
                drop_stale(&mut bursts, now);
                if let Some(burst) = bursts.iter().find(|burst| burst.ship == ship) {
                    return Some(if now >= burst.next { high } else { low });
                }
            }
        }
        None
    }))
    .ok()
    .flatten();
    if let Some(result) = override_roll {
        return result;
    }

    let table = (engine as *const *const usize).read_unaligned();
    let random: unsafe extern "C" fn(*mut u8, f32, f32) -> f32 =
        std::mem::transmute(table.byte_add(RANDOM_SLOT).read_unaligned());
    random(engine, low, high)
}

/// Launches from the launcher like the game does, but for an eligible AI ship it reports a launch as a
/// failure with the configured chance, and starts a burst, so the ship launches again soon.
unsafe extern "C" fn launch_hook(launcher: *mut u8) -> u64 {
    let launch: unsafe extern "C" fn(*mut u8) -> u64 = std::mem::transmute(LAUNCH);

    let owner = catch_unwind(AssertUnwindSafe(|| {
        let launcher_addr = launcher as usize;
        if !plausible(launcher_addr) {
            return None;
        }
        let ship = rd::<usize>(launcher_addr + OWNER_OFFSET)?;
        eligible_ship(ship).then_some(ship)
    }))
    .ok()
    .flatten();
    let Some(ship) = owner else {
        return launch(launcher);
    };
    let Some(now) = game_time() else {
        return launch(launcher);
    };

    // Later launchers of the same tick wait for their turn in the burst.
    if bursts()
        .iter()
        .any(|burst| burst.ship == ship && now < burst.next)
    {
        return 0;
    }

    let result = launch(launcher);
    if result & 0xff == 0 {
        return result;
    }

    let suppress = catch_unwind(AssertUnwindSafe(|| {
        let mut bursts = bursts();
        bursts.retain(|burst| burst.ship != ship);
        if rng::random_f32() < f32::from_bits(DUMP_CHANCE.load(Ordering::Relaxed)) {
            let (min_delay, max_delay) = delay_range();
            let delay = min_delay + (max_delay - min_delay) * f64::from(rng::random_f32());
            bursts.push(Burst {
                ship,
                next: now + delay,
                expires: now + delay + BURST_TIMEOUT,
            });
            log::debug!("hf_ai: missile dump: ship {ship:#x} launched, next launch in {delay:.2} s");
            return true;
        }
        false
    }))
    .unwrap_or(false);
    if suppress {
        return 0;
    }
    result
}
