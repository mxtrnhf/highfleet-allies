//! Native AI-task attribution spans: per-`aiShipDriver` call and per
//! `AiShip_ScanObstacles` call, timed at the native call sites.
//!
//! The 2026-11-08 captures show `ai_us` superlinear in AI-list length (79 µs at
//! one entry, 39 ms at four; `docs/ai-plateau-attribution-2026-11-09.md`). The
//! obstacle scan rebuilds a sorted, shadow-pruned linked list of nearby ship
//! parts every AI tick, so the leading suspect is its insertion walk. These
//! spans partition the disputed time without touching behaviour:
//!
//! - `ai_driver`: the whole per-entry `aiShipDriver` call from the
//!   `SomethingAITask` loop (inclusive: tick, scans, threat list, weapons).
//! - `ai_scan`: the `AiShip_ScanObstacles` calls from `AiShipTick` and
//!   `AiShip_SteerController` (the part-recursion walk). The previous tick's
//!   list free loop is inlined in `AiShipTick` and stays inside `ai_driver`.
//!
//! The call sites are wrapped, not the functions, so every argument register and
//! stack argument reaches the original callee untouched: a `preserved_call`
//! around a no-argument Rust timing hook clobbers only volatile state, which a
//! normal call may clobber anyway, and `absolute_call` performs the original
//! call with the caller's frame intact. The replaced relative `CALL` is not
//! replayed; the trampoline performs the equivalent absolute call.
//!
//! Overhead per span is two QPC reads and two atomic adds; `ai_spans: false` in
//! `hf_perf.json` disables the detours so the instrumentation cost can be A/B'd.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use patchy::{PatchSession, ReturnType, Trampoline};

use crate::profile;

const IMAGE_BASE: usize = 0x1_4000_0000;

/// `CALL aiShipDriver` in the `SomethingAITask` loop; the loop ignores the
/// return value (`TEST AL,AL` belongs to the earlier virtual gate).
const DRIVER_SITE: usize = IMAGE_BASE + 0x11_23a;
const DRIVER_TARGET: usize = IMAGE_BASE + 0x73_9c0;
const DRIVER_RESUME: usize = IMAGE_BASE + 0x11_23f;
const DRIVER_CALL: [u8; 5] = [0xe8, 0x81, 0x27, 0x06, 0x00];

/// `CALL AiShip_ScanObstacles` in `AiShipTick` (the inlined avoidance path that
/// runs for driver states 1/2).
const TICK_SCAN_SITE: usize = IMAGE_BASE + 0x64_e6f;
const TICK_SCAN_RESUME: usize = IMAGE_BASE + 0x64_e74;
const TICK_SCAN_CALL: [u8; 5] = [0xe8, 0xac, 0x7d, 0x1e, 0x00];

/// `CALL AiShip_ScanObstacles` in `AiShip_SteerController` (driver states 0/4).
const CTRL_SCAN_SITE: usize = IMAGE_BASE + 0x6c_f83;
const CTRL_SCAN_RESUME: usize = IMAGE_BASE + 0x6c_f88;
const CTRL_SCAN_CALL: [u8; 5] = [0xe8, 0x98, 0xfc, 0x1d, 0x00];

const SCAN_TARGET: usize = IMAGE_BASE + 0x24_cc20;

static INSTALLED: AtomicBool = AtomicBool::new(false);
static DRIVER_TICKS: AtomicU64 = AtomicU64::new(0);
static DRIVER_CALLS: AtomicU64 = AtomicU64::new(0);
static SCAN_TICKS: AtomicU64 = AtomicU64::new(0);
static SCAN_CALLS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// QPC of the enclosing `aiShipDriver` call; drivers never nest.
    static DRIVER_START: Cell<i64> = const { Cell::new(0) };
    /// QPC of the enclosing `AiShip_ScanObstacles` call; the two scan call
    /// sites are never active at the same time on the single game thread.
    static SCAN_START: Cell<i64> = const { Cell::new(0) };
}

/// Whether the span detours were installed (columns are empty otherwise).
pub fn installed() -> bool {
    INSTALLED.load(Ordering::Relaxed)
}

/// Takes the accumulated driver span since the previous frame.
pub fn take_driver() -> (u64, u64) {
    (
        DRIVER_TICKS.swap(0, Ordering::Relaxed),
        DRIVER_CALLS.swap(0, Ordering::Relaxed),
    )
}

/// Takes the accumulated scan span since the previous frame.
pub fn take_scan() -> (u64, u64) {
    (
        SCAN_TICKS.swap(0, Ordering::Relaxed),
        SCAN_CALLS.swap(0, Ordering::Relaxed),
    )
}

unsafe extern "C" fn driver_enter() {
    DRIVER_START.with(|start| start.set(profile::clock()));
}

unsafe extern "C" fn driver_exit() {
    let begin = DRIVER_START.with(Cell::get);
    let elapsed = (profile::clock() - begin).max(0);
    DRIVER_TICKS.fetch_add(elapsed as u64, Ordering::Relaxed);
    DRIVER_CALLS.fetch_add(1, Ordering::Relaxed);
}

unsafe extern "C" fn scan_enter() {
    SCAN_START.with(|start| start.set(profile::clock()));
}

unsafe extern "C" fn scan_exit() {
    let begin = SCAN_START.with(Cell::get);
    let elapsed = (profile::clock() - begin).max(0);
    SCAN_TICKS.fetch_add(elapsed as u64, Ordering::Relaxed);
    SCAN_CALLS.fetch_add(1, Ordering::Relaxed);
}

/// Whether every wrapped call site still has the analysed bytes.
///
/// # Safety
///
/// The caller must be in the startup-only installation path with the supported
/// 1.163 image mapped at the fixed base. This reads executable bytes only.
pub unsafe fn validate() -> Result<(), String> {
    for (site, expected) in [
        (DRIVER_SITE, DRIVER_CALL.as_slice()),
        (TICK_SCAN_SITE, TICK_SCAN_CALL.as_slice()),
        (CTRL_SCAN_SITE, CTRL_SCAN_CALL.as_slice()),
    ] {
        // SAFETY: bounded code ranges of the supported fixed-base image.
        let found = unsafe { std::slice::from_raw_parts(site as *const u8, expected.len()) };
        if found != expected {
            return Err(format!(
                "AI call site differs at {site:#x}; refusing AI span hooks"
            ));
        }
    }
    Ok(())
}

/// Wraps one native `CALL target` at `site` with entry/exit timing.
///
/// The trampoline preserves the caller's register and stack state around the
/// timing hooks (`preserved_call` saves and restores every volatile register),
/// then performs the original call with the caller's frame untouched, records
/// the exit time, and resumes at `resume`.
unsafe fn wrap_call(
    session: &mut PatchSession,
    site: usize,
    target: usize,
    resume: usize,
    enter: unsafe extern "C" fn(),
    exit: unsafe extern "C" fn(),
) -> Result<(), String> {
    let mut trampoline = Trampoline::new();
    // The timing hooks take no arguments and return nothing; the preserved
    // wrapper restores every volatile register around them.
    trampoline.preserved_call(enter as *const (), &[], &[], ReturnType::None);
    // The original call: argument registers and stack arguments are exactly
    // what the caller set up; the shadow space and stack arguments stay in
    // the caller's frame above RSP, which the wrapper never touches.
    trampoline.absolute_call(target as *const ());
    trampoline.preserved_call(exit as *const (), &[], &[], ReturnType::None);
    trampoline.absolute_jump(resume);
    // SAFETY: the replaced 5-byte relative CALL is a complete instruction; the
    // trampoline performs the equivalent call itself, so nothing is replayed.
    unsafe {
        session
            .detour_trampoline(site, 5, trampoline)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Installs the AI span hooks. Fails closed: any mismatch leaves everything
/// unpatched and the span columns empty.
///
/// # Safety
///
/// Startup-only installation with the supported image mapped at the fixed
/// base, before any scene runs. The caller must retain the DLL for process
/// lifetime and install the patch session permanently afterwards.
pub unsafe fn prepare(session: &mut PatchSession) -> Result<(), String> {
    unsafe { validate()? };
    wrap_call(
        session,
        DRIVER_SITE,
        DRIVER_TARGET,
        DRIVER_RESUME,
        driver_enter,
        driver_exit,
    )?;
    wrap_call(
        session,
        TICK_SCAN_SITE,
        SCAN_TARGET,
        TICK_SCAN_RESUME,
        scan_enter,
        scan_exit,
    )?;
    wrap_call(
        session,
        CTRL_SCAN_SITE,
        SCAN_TARGET,
        CTRL_SCAN_RESUME,
        scan_enter,
        scan_exit,
    )?;
    INSTALLED.store(true, Ordering::Release);
    log::info!(
        "hf_perf: AI spans installed: ai_driver (aiShipDriver 0x1400739c0) and ai_scan \
         (AiShip_ScanObstacles 0x14024cc20, both call sites)"
    );
    Ok(())
}
