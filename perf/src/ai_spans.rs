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
//! Native CALLs target frameless tail-jump relays. Typed C-unwind wrappers pass
//! all native arguments and return values through unchanged, with normal DLL
//! unwind information. Do not restore the old JMP/CALL/JMP trampoline: its
//! anonymous CALL return address made the native exception chain unwalkable.
//!
//! Overhead per span is two QPC reads and two atomic adds; `ai_spans: false` in
//! `hf_perf.json` disables the detours so the instrumentation cost can be A/B'd.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use patchy::PatchSession;

use crate::{call_hook, profile};

const IMAGE_BASE: usize = 0x1_4000_0000;

/// `CALL aiShipDriver` in the `SomethingAITask` loop; the loop ignores the
/// return value (`TEST AL,AL` belongs to the earlier virtual gate).
const DRIVER_SITE: usize = IMAGE_BASE + 0x11_23a;
const DRIVER_TARGET: usize = IMAGE_BASE + 0x73_9c0;
const DRIVER_CALL: [u8; 5] = [0xe8, 0x81, 0x27, 0x06, 0x00];

/// `CALL AiShip_ScanObstacles` in `AiShipTick` (the inlined avoidance path that
/// runs for driver states 1/2).
const TICK_SCAN_SITE: usize = IMAGE_BASE + 0x64_e6f;
const TICK_SCAN_CALL: [u8; 5] = [0xe8, 0xac, 0x7d, 0x1e, 0x00];

/// `CALL AiShip_ScanObstacles` in `AiShip_SteerController` (driver states 0/4).
const CTRL_SCAN_SITE: usize = IMAGE_BASE + 0x6c_f83;
const CTRL_SCAN_CALL: [u8; 5] = [0xe8, 0x98, 0xfc, 0x1d, 0x00];

const SCAN_TARGET: usize = IMAGE_BASE + 0x24_cc20;

static DRIVER_INSTALLED: AtomicBool = AtomicBool::new(false);
static SCAN_INSTALLED: AtomicBool = AtomicBool::new(false);
static DRIVER_TICKS: AtomicU64 = AtomicU64::new(0);
static DRIVER_CALLS: AtomicU64 = AtomicU64::new(0);
static SCAN_TICKS: AtomicU64 = AtomicU64::new(0);
static SCAN_CALLS: AtomicU64 = AtomicU64::new(0);

/// Driver and scan coverage are independent; absent columns must stay blank.
pub fn driver_installed() -> bool {
    DRIVER_INSTALLED.load(Ordering::Relaxed)
}

pub fn scan_installed() -> bool {
    SCAN_INSTALLED.load(Ordering::Relaxed)
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

fn record(start: i64, ticks: &AtomicU64, calls: &AtomicU64) {
    let Some(elapsed) = profile::clock()
        .checked_sub(start)
        .filter(|elapsed| *elapsed >= 0)
    else {
        return;
    };
    ticks.fetch_add(elapsed as u64, Ordering::Relaxed);
    calls.fetch_add(1, Ordering::Relaxed);
}

unsafe extern "C-unwind" fn timed_driver(creature: *mut c_void) -> u8 {
    let start = profile::clock();
    // SAFETY: Guarded direct CALL has RCX=creature, native AL return. The
    // wrapper borrows no game-owned fields and returns native's AL unchanged.
    let native: unsafe extern "C-unwind" fn(*mut c_void) -> u8 =
        unsafe { std::mem::transmute(DRIVER_TARGET) };
    let result = unsafe { native(creature) };
    record(start, &DRIVER_TICKS, &DRIVER_CALLS);
    result
}

unsafe extern "C-unwind" fn timed_scan(
    creature: *mut c_void,
    list: *mut c_void,
    range: f32,
    min_mass: f32,
    max_explosive: f32,
) {
    let start = profile::clock();
    // SAFETY: Supported native signature verified at both callers and callee:
    // RCX, RDX, XMM2, XMM3, fifth f32 on stack, void return. The list remains
    // borrowed game storage; native retains all allocation/traversal/ownership.
    // C-unwind plus an actual CALL return address preserves foreign unwinding.
    let native: unsafe extern "C-unwind" fn(*mut c_void, *mut c_void, f32, f32, f32) =
        unsafe { std::mem::transmute(SCAN_TARGET) };
    unsafe { native(creature, list, range, min_mass, max_explosive) };
    // An escaping native exception contributes no completed timing sample.
    record(start, &SCAN_TICKS, &SCAN_CALLS);
}

/// Whether every call site in one optional coverage group has analysed bytes.
///
/// # Safety
///
/// The caller must be in the startup-only installation path with the supported
/// 1.163 image mapped at the fixed base. This reads executable bytes only.
unsafe fn validate_sites(sites: &[(usize, [u8; 5])]) -> Result<(), String> {
    for &(site, expected) in sites {
        // SAFETY: bounded code ranges of the supported fixed-base image.
        let found = unsafe { std::slice::from_raw_parts(site as *const u8, expected.len()) };
        if found != expected {
            return Err(format!("AI call site differs at {site:#x}"));
        }
    }
    Ok(())
}

/// Prepare independent driver and obstacle-scan groups. A byte conflict skips
/// only its group; both scan sites must match before either is prepared. Any
/// preparation/allocation error aborts the caller's whole uninstalled session.
///
/// # Safety
///
/// Startup-only installation with the supported image mapped at the fixed
/// base, before any scene runs. The caller must retain the DLL for process
/// lifetime and install the patch session permanently afterwards.
pub unsafe fn prepare(session: &mut PatchSession) -> Result<(), String> {
    // SAFETY: Caller supplies the fixed-image, quiescent startup contract.
    let driver = match unsafe { validate_sites(&[(DRIVER_SITE, DRIVER_CALL)]) } {
        Ok(()) => true,
        Err(error) => {
            log::warn!("hf_perf: optional AI driver span unavailable: {error}; driver site untouched, driver columns blank; core profiling continues");
            false
        }
    };
    let scans = match unsafe {
        validate_sites(&[
            (TICK_SCAN_SITE, TICK_SCAN_CALL),
            (CTRL_SCAN_SITE, CTRL_SCAN_CALL),
        ])
    } {
        Ok(()) => true,
        Err(error) => {
            log::warn!("hf_perf: optional AI scan spans unavailable: {error}; both scan sites untouched, scan columns blank; core profiling continues");
            false
        }
    };
    if driver {
        unsafe {
            call_hook::prepare_call(
                session,
                DRIVER_SITE,
                &DRIVER_CALL,
                timed_driver as *const (),
            )?
        };
    }
    if scans {
        unsafe {
            call_hook::prepare_call(
                session,
                TICK_SCAN_SITE,
                &TICK_SCAN_CALL,
                timed_scan as *const (),
            )?;
            call_hook::prepare_call(
                session,
                CTRL_SCAN_SITE,
                &CTRL_SCAN_CALL,
                timed_scan as *const (),
            )?;
        }
    }
    DRIVER_INSTALLED.store(driver, Ordering::Release);
    SCAN_INSTALLED.store(scans, Ordering::Release);
    log::info!(
        "hf_perf: AI spans prepared: driver={driver}, scans={scans}; scan time covers obstacle calls only, excludes previous-list free; nested within ai_us, not additive"
    );
    Ok(())
}
