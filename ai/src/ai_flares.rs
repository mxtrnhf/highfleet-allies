//! Lets AI ships panic against incoming missiles: dump flares and evade (Steam 1.163 only).
//!
//! **hf_ai port** of `hf_qol_fork/src/ai_flares.rs` (fork branch `ai-flares`, commit 5c3ab73), behaviour-identical.
//! The text below is the original description, kept as is. What changed in this port: the steering hook at
//! `0x140064a8d` and its scratch buffer belong to `lib.rs` (always installed there, log-only without ai_flares), which
//! calls [`steer`] after the waypoint stage `decide()`; the timers and direction math live in `panic_math.rs`
//! (pure, tested on the host); game-memory reads are page-checked; the hooks are wrapped in `catch_unwind`;
//! `log_panic` writes a per-ship event trace. Exactly one of qol `ai_flares` and hf_ai `ai_flares` may be enabled:
//! both patch the same three sites, and a second loader finds other bytes and skips (hf_ai logs an error line).
//!
//! The game's AI ships carry flare launchers, and the release routine is generic, but nothing on
//! the AI side ever asks for a release: the per-creature update only fires the launchers when the
//! byte `creature + 0xbc0` is set, and only the player's input sets it. Setting that byte for an
//! AI ship is not an option, because the same branch also redraws the player's flare HUD from the
//! ship's first launcher. So the hook calls the release routine itself.
//!
//! The hook is the `cmp byte [rsi+0xbc0], 0` at `0x14007f864` in the per-creature update
//! (`rsi` = creature, 7 bytes, stack aligned there). For every AI creature it:
//!
//! 1. keeps firing all launchers while a burst is running;
//! 2. otherwise, at most ten times a second, looks for a missile that is locked on to the
//!    creature and closer than `trigger_range`; if one is found it rolls `reaction_probability`
//!    and on success starts a burst of `burst_seconds`, followed by `min_interval_seconds` of quiet.
//!
//! While a panic runs the ship also evades. The AI tick (`0x140062f90`) ends its steering by
//! storing a desired waypoint, 200 units from the ship along the wanted direction, at
//! `creature + 0xbf0/0xbf4` (`movss [r14+0xbf0], xmm9` at `0x140064a8d`; `xmm9/xmm10` = waypoint
//! x/y, `xmm12/xmm14` = ship x/y), and then steers to it. A second hook there pulls that wanted
//! direction towards a sidestep perpendicular to the missile's line of approach, with some
//! retreat mixed in, and keeps to one randomly chosen side. A rare *barrel roll* (a dice roll when
//! the panic starts) replaces the evasion with what the player's Q/E plus Shift does: the engine
//! commands are rewritten so the two engine groups thrust in opposite directions and the ship spins.
//!
//! A *panic evasion* (a dice roll when the panic starts) also boosts the engines. The player's
//! Shift key sets `creature + 0xba8` to 2.0, and the player control code copies it into the
//! strength field (`+4`) of the engine commands `0x66` and `0x67`. The AI tick builds the same
//! commands on its stack (`rbp + 0x28` and `rbp + 0x48`, 0x20 bytes apart, strength at `+4`,
//! limited to 1.0), so a third hook at `0x140065744`, just before they are sent to the engines,
//! raises the strength to the configured boost.
//!
//! Each launcher still obeys its own rate and ammunition gates inside the release routine, so a
//! burst drops about one flare per launcher per launcher interval.
//!
//! Addresses and offsets (Steam 1.163, image base `0x140000000`):
//!
//! | What                                         | Where                                    |
//! |----------------------------------------------|------------------------------------------|
//! | flare hook site / resume                     | `0x14007f864` / `0x14007f86b`            |
//! | steering hook site / resume                  | `0x140064a8d` / `0x140064a96`            |
//! | boost hook site / resume                     | `0x140065744` / `0x140065750`            |
//! | release routine `(launcher) -> fired`        | `0x14002eed0`                            |
//! | stats lookup `(id) -> record or null`        | `0x140281e00` (category at `record+0xb0`)|
//! | body to body distance `(a, b) -> float`      | `0x14020fb80`                            |
//! | body pointer array / count (`u32`)           | `0x143a33c90` / `0x143a51150`            |
//! | game clock (`f64`, seconds)                  | `0x143a119c0`                            |
//! | creature: side float / launcher vector       | `+0xad4` (negative = AI) / `+0x1408..0x1410`, 0x18-byte entries, launcher at `+8` |
//! | launcher: cooldown / count / rate timer      | `+0x848` / `+0x860` / `+0x86c` (floats)  |
//! | body: stats id / target link pointer         | `+0x188` / `+0x498`; link owner `+0xb8`  |
//! | body: position x, y                          | `+0x128`, `+0x12c` (floats)              |

use std::{
    collections::HashMap,
    ffi::c_void,
    fs::File,
    io::Write as _,
    panic::{catch_unwind, AssertUnwindSafe},
    ptr,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Mutex, OnceLock,
    },
};

use patchy::{PatchSession, ProcessModule, Result, ReturnType, Trampoline};
use windows_sys::Win32::System::Memory::{VirtualQuery, MEMORY_BASIC_INFORMATION};

use crate::{
    flares_config::AiFlaresConfig,
    curtain_math::{self, Curtain},
    panic_math::{self, Params, ScanEvent, Ship, Threat},
    rng, STEER_RVA,
};

const HOOK_RVA: usize = 0x7f864;
const RESUME_RVA: usize = 0x7f86b;
/// `cmp byte [rsi+0xbc0], 0` followed by the `je` that consumes its flags.
const HOOK_BYTES: [u8; 13] = [
    0x80, 0xbe, 0xc0, 0x0b, 0x00, 0x00, 0x00, 0x0f, 0x84, 0xee, 0x00, 0x00, 0x00,
];
/// The 7 bytes that are replaced (the `cmp`).
const STOLEN_LEN: usize = 7;
/// `mov rcx, rsi`: the hook's only argument is the creature.
const ARGUMENT_SETUP: [u8; 3] = [0x48, 0x8b, 0xce];

const BOOST_RVA: usize = 0x65744;
const BOOST_RESUME_RVA: usize = 0x65750;
/// `mov rdi, rbx`, a 9-byte `nop`, and the first instruction of the engine command loop.
const BOOST_BYTES: [u8; 17] = [
    0x48, 0x8b, 0xfb, 0x66, 0x0f, 0x1f, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00, 0x49, 0x8b, 0x4c, 0x38,
    0x08,
];
const BOOST_STOLEN_LEN: usize = 12;
/// `mov c7 45 28 66 00 00 00`: the `0x66` engine command being set up, a few lines above.
const BOOST_COMMAND_RVA: usize = 0x656d9;
const BOOST_COMMAND_BYTES: [u8; 7] = [0xc7, 0x45, 0x28, 0x66, 0x00, 0x00, 0x00];
/// Offsets from the `0x66` command: the strength of it and of the `0x67` command after it.
const COMMAND_STRENGTH: usize = 4;
const COMMAND_NEXT_STRENGTH: usize = 0x24;
const RELEASE_RVA: usize = 0x2eed0;
const RELEASE_PROLOGUE: [u8; 8] = [0x48, 0x8b, 0xc4, 0x53, 0x55, 0x56, 0x57, 0x48];
/// `traverseLinkedListCreature_(creature) -> body`: the body that carries the ship's heading.
const CREATURE_BODY_RVA: usize = 0x48fc0;
const CREATURE_BODY_PROLOGUE: [u8; 10] = [0x48, 0x8b, 0xd1, 0x48, 0x8b, 0x89, 0x88, 0x00, 0x00, 0x00];
/// Body heading in radians.
const BODY_HEADING: usize = 0x138;
/// Offsets from the `0x66` command: the angle of it and of the `0x67` command after it, and the
/// angles of the `0x64` and `0x65` commands before it.
const COMMAND_ANGLE: usize = 0x10;
const COMMAND_NEXT_ANGLE: usize = 0x30;
const COMMAND_PREV2_ANGLE: isize = -0x3c;
const COMMAND_PREV_ANGLE: isize = -0x1c;
const STATS_RVA: usize = 0x281e00;
const STATS_PROLOGUE: [u8; 8] = [0x83, 0xf9, 0xff, 0x74, 0x22, 0x48, 0x8b, 0x15];
const DISTANCE_RVA: usize = 0x20fb80;
const DISTANCE_PROLOGUE: [u8; 8] = [0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b];
/// The per-creature update's own first bytes, a check that the hook site is in the right function.
const UPDATE_RVA: usize = 0x7eb50;
const UPDATE_PROLOGUE: [u8; 7] = [0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18];

const BODIES_RVA: usize = 0x3a33c90;
const BODY_COUNT_RVA: usize = 0x3a51150;
const CLOCK_RVA: usize = 0x3a119c0;

const CREATURE_SIDE: usize = 0xad4;
/// Offset in a ship of the AI flag: non-zero when the ship is not flown by the player.
const CREATURE_AI_FLAG: usize = 0xbd4;
const CREATURE_LAUNCHERS_BEGIN: usize = 0x1408;
const CREATURE_LAUNCHERS_END: usize = 0x1410;
const LAUNCHER_ENTRY_SIZE: usize = 0x18;
const LAUNCHER_PTR: usize = 0x8;
/// Offset in a launcher of its loaded count (float).
const LAUNCHER_COUNT: usize = 0x860;
const BODY_POS_X: usize = 0x128;
const BODY_POS_Y: usize = 0x12c;
const BODY_STATS_ID: usize = 0x188;
const BODY_TARGET: usize = 0x498;
const BODY_OWNER: usize = 0xb8;
const STATS_CATEGORY: usize = 0xb0;
const VTABLE_IS_ACTIVE: usize = 0x78;

/// Stats categories of guided missiles (tactical and strategic).
const CATEGORY_TACTICAL_MISSILE: u32 = 0x4000;
const CATEGORY_STRATEGIC_LAUNCH: u32 = 0x800_0000;

/// Per-creature state is dropped once the table grows past this and the entries are idle.
const PRUNE_THRESHOLD: usize = 128;
/// hf_ai hardening (the original loops are unbounded): most bodies / launchers a scan will look at.
const MAX_BODIES: usize = 65_536;
const MAX_LAUNCHERS: usize = 256;
/// Seconds between two trace lines of the same kind for one ship (steering and engine hooks run every frame).
const TRACE_PERIOD: f64 = 0.5;

/// Creature centre of mass (same fields `lib.rs` reads), for the trace.
/// Ship bounding radius (`Creature +0x1850`).
const CREATURE_RADIUS: usize = 0x1850;
const CREATURE_POS_X: usize = 0x1840;
const CREATURE_POS_Y: usize = 0x1844;
/// qol's flare-crash fix site in `creatureLogic` (`MOV RBX,RAX; MOV RCX,RSI`); still these bytes = not patched.
const FLARE_CRASH_RVA: usize = 0x36bb5;
const FLARE_CRASH_ORIGINAL: [u8; 6] = [0x48, 0x8b, 0xd8, 0x48, 0x8b, 0xce];

type ReleaseFn = unsafe extern "C" fn(*mut u8) -> u8;
type CreatureBodyFn = unsafe extern "C" fn(*const u8) -> usize;
type StatsFn = unsafe extern "C" fn(i32) -> *const u8;
type DistanceFn = unsafe extern "C" fn(*const u8, *const u8) -> f32;
type IsActiveFn = unsafe extern "C" fn(*const u8) -> u8;

struct Runtime {
    config: AiFlaresConfig,
    params: Params,
    curtain: curtain_math::Params,
    release: ReleaseFn,
    stats: StatsFn,
    creature_body: CreatureBodyFn,
    distance: DistanceFn,
    bodies: usize,
    body_count: usize,
    clock: usize,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

#[derive(Default)]
struct Ships {
    last_clock: f64,
    by_creature: HashMap<usize, Ship>,
    curtains: HashMap<usize, Curtain>,
}

static SHIPS: OnceLock<Mutex<Ships>> = OnceLock::new();

static LOGGED_STRAY: AtomicBool = AtomicBool::new(false);
static LOGGED_DIRECT_TARGET: AtomicBool = AtomicBool::new(false);
static LOGGED_CHILD_TARGET: AtomicBool = AtomicBool::new(false);
/// How many hook panics were caught (only the first few are logged).
static CAUGHT_PANICS: AtomicU32 = AtomicU32::new(0);
static LOGGED_CRASH_FIX_CHECK: AtomicBool = AtomicBool::new(false);

/// hf_ai: reads `T` at `address` only if the pages are mapped and readable (same test as `hf_ally`'s `diag::rd`).
/// The raw `read` of the original is used only for the AI tick's own arguments (see [`Region`] for the scan).
unsafe fn rd<T: Copy>(address: usize) -> Option<T> {
    let end = address.checked_add(std::mem::size_of::<T>())?;
    if address < 0x1_0000 || !readable(address) || (end - 1) / 4096 != address / 4096 && !readable(end - 1) {
        return None;
    }
    Some(ptr::read_unaligned(address as *const T))
}

unsafe fn readable(address: usize) -> bool {
    region_of(address).is_some()
}

/// The committed, accessible memory region around `address` as `(start, end)`.
unsafe fn region_of(address: usize) -> Option<(usize, usize)> {
    let mut info: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
    if VirtualQuery(address as *const c_void, &mut info, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) == 0 {
        return None;
    }
    // MEM_COMMIT, and not PAGE_NOACCESS (0x01) / PAGE_GUARD (0x100)
    if info.State == 0x1000 && info.Protect & 0x101 == 0 && info.Protect != 0 {
        let start = info.BaseAddress as usize;
        Some((start, start.checked_add(info.RegionSize)?))
    } else {
        None
    }
}

/// Page-checked reads for the body scan: remembers the last few regions the scan touched, so a body is checked
/// without a `VirtualQuery` per field (the scan reads every body, ten times a second, for every AI ship).
/// A value that straddles two regions reads as `None`.
struct Region {
    cached: [(usize, usize); 4],
    next: usize,
}

impl Region {
    fn new() -> Self {
        Self { cached: [(0, 0); 4], next: 0 }
    }

    unsafe fn read<T: Copy>(&mut self, address: usize) -> Option<T> {
        let end = address.checked_add(std::mem::size_of::<T>())?;
        if address < 0x1_0000 {
            return None;
        }
        let known = self.cached.iter().any(|&(start, stop)| address >= start && end <= stop);
        if !known {
            let region = region_of(address)?;
            if end > region.1 {
                return None;
            }
            self.cached[self.next] = region;
            self.next = (self.next + 1) % self.cached.len();
        }
        Some(ptr::read_unaligned(address as *const T))
    }
}

/// Checks that `rva` starts with `expected`.
unsafe fn code_matches(module: ProcessModule, rva: usize, expected: &[u8]) -> Result<bool> {
    let address = module.resolve_rva(rva)?;
    Ok(std::slice::from_raw_parts(address as *const u8, expected.len()) == expected)
}

/// Appended to the error line when a site has other bytes (used by `lib.rs` for the steering site too).
pub const OWNERSHIP_HINT: &str =
    "another mod already patched it; qol `ai_flares.enabled` and hf_ai `ai_flares.enabled` must not both be on";

/// Prepares the AI panic hooks (flare site, and the boost site when needed; the steering site is `lib.rs`'s).
/// `steering_hook_ok` says whether `lib.rs` found the steering code as expected and prepared its hook.
/// A site whose game code is not what was analysed is skipped, with the reason in the log.
pub unsafe fn patch_ai_flares(
    session: &mut PatchSession,
    config: &AiFlaresConfig,
    steering_hook_ok: bool,
) -> Result {
    let module = ProcessModule::main()?;
    INCLUDE_ALLIES.store(config.allies, Ordering::Relaxed);

    let sites: [(&str, usize, &[u8]); 5] = [
        ("creature body", CREATURE_BODY_RVA, &CREATURE_BODY_PROLOGUE),
        ("creature update", UPDATE_RVA, &UPDATE_PROLOGUE),
        ("flare release", RELEASE_RVA, &RELEASE_PROLOGUE),
        ("stats lookup", STATS_RVA, &STATS_PROLOGUE),
        ("distance", DISTANCE_RVA, &DISTANCE_PROLOGUE),
    ];
    for (name, rva, expected) in sites {
        if !code_matches(module, rva, expected)? {
            log::error!(
                "hf_ai ai_flares: the {name} code at {rva:#x} is not what was expected, ai_flares stays OFF"
            );
            return Ok(());
        }
    }
    let flare_hook_ok = code_matches(module, HOOK_RVA, &HOOK_BYTES)?;
    if !flare_hook_ok {
        log::error!(
            "hf_ai ai_flares: the flare hook site at {HOOK_RVA:#x} has other bytes ({OWNERSHIP_HINT}), ai_flares stays OFF"
        );
        return Ok(());
    }

    let mut config = config.sanitized();
    let steer_hook_ok = steering_hook_ok;
    if config.evade && !steer_hook_ok {
        log::error!(
            "hf_ai ai_flares: the steering hook at {STEER_RVA:#x} is not installed, evasion stays off"
        );
        config.evade = false;
    }

    let boost_hook_ok = steer_hook_ok
        && code_matches(module, BOOST_RVA, &BOOST_BYTES)?
        && code_matches(module, BOOST_COMMAND_RVA, &BOOST_COMMAND_BYTES)?;
    let boost = config.evade
        && ((config.panic_boost > 1.0 && config.panic_evasion_probability > 0.0)
            || config.barrel_roll_probability > 0.0)
        || crate::tactics::wants_dive_boost()
        || (config.evade && config.launch_flares_probability > 0.0 && config.launch_flares_evade > 0.0 && config.launch_flares_boost > 1.0);
    if boost && !boost_hook_ok {
        log::error!(
            "hf_ai ai_flares: the engine command code at {BOOST_RVA:#x} has other bytes ({OWNERSHIP_HINT}), the panic boost stays off"
        );
        config.panic_boost = 1.0;
    }
    let boost = boost && boost_hook_ok;

    let runtime = Runtime {
        config: config.clone(),
        params: Params {
            evade: config.evade,
            reaction_probability: config.reaction_probability,
            burst_seconds: config.burst_seconds,
            min_interval_seconds: config.min_interval_seconds,
            evasion_seconds: config.evasion_seconds,
            evasion_strength: config.evasion_strength,
            barrel_roll_probability: config.barrel_roll_probability,
            barrel_roll_seconds: config.barrel_roll_seconds,
            panic_evasion_probability: config.panic_evasion_probability,
            panic_evasion_seconds: config.panic_evasion_seconds,
        },
        curtain: curtain_math::Params {
            probability: config.launch_flares_probability,
            interval_min: config.launch_flares_interval_min,
            interval_max: config.launch_flares_interval_max,
            seconds: config.launch_flares_seconds,
            max_drops: config.launch_flares_max_drops,
            cooldown: config.launch_flares_cooldown,
            delay_min: config.launch_flares_delay_min,
            delay_max: config.launch_flares_delay_max,
            eta_lead: config.launch_flares_eta_lead,
            eta_max_wait: config.launch_flares_eta_max_wait,
        },
        release: std::mem::transmute::<usize, ReleaseFn>(module.resolve_rva(RELEASE_RVA)?),
        creature_body: std::mem::transmute::<usize, CreatureBodyFn>(module.resolve_rva(CREATURE_BODY_RVA)?),
        stats: std::mem::transmute::<usize, StatsFn>(module.resolve_rva(STATS_RVA)?),
        distance: std::mem::transmute::<usize, DistanceFn>(module.resolve_rva(DISTANCE_RVA)?),
        bodies: module.resolve_rva(BODIES_RVA)?,
        body_count: module.resolve_rva(BODY_COUNT_RVA)?,
        clock: module.resolve_rva(CLOCK_RVA)?,
    };
    if RUNTIME.set(runtime).is_err() {
        log::error!("hf_ai ai_flares: already prepared");
        return Ok(());
    }

    rng::seed();

    // The steering detour is prepared by `lib.rs` (always, log-only without ai_flares).
    session.detour_trampoline(
        module.resolve_rva(HOOK_RVA)?,
        STOLEN_LEN,
        build_trampoline(module)?,
    )?;
    if boost {
        session.detour_trampoline(
            module.resolve_rva(BOOST_RVA)?,
            BOOST_STOLEN_LEN,
            build_boost_trampoline(module)?,
        )?;
    }

    log::info!(
        "hf_ai ai_flares prepared: range {}, reaction {}, flares {} s, interval {} s, evasion {} (strength {}, {} s), panic evasion {} x{} for {} s, barrel roll {} for {} s, allies {}, trace {}",
        config.trigger_range,
        config.reaction_probability,
        config.burst_seconds,
        config.min_interval_seconds,
        if config.evade { "on" } else { "off" },
        config.evasion_strength,
        config.evasion_seconds,
        config.panic_evasion_probability,
        if boost { config.panic_boost } else { 1.0 },
        config.panic_evasion_seconds,
        config.barrel_roll_probability,
        config.barrel_roll_seconds,
        config.allies,
        config.log_panic
    );
    if config.launch_flares_probability > 0.0 {
        log::info!(
            "hf_ai ai_flares: flare curtain ON (chance {}, every {}..{} s for {} s, at most {} drops, cooldown {} s)",
            config.launch_flares_probability,
            config.launch_flares_interval_min,
            config.launch_flares_interval_max,
            config.launch_flares_seconds,
            config.launch_flares_max_drops,
            config.launch_flares_cooldown
        );
    }
    Ok(())
}

fn build_trampoline(module: ProcessModule) -> Result<Trampoline> {
    let mut trampoline = Trampoline::new();

    trampoline.preserved_call(
        on_creature_update as *const (),
        &ARGUMENT_SETUP,
        &[],
        ReturnType::None,
    );
    // Replay the replaced `cmp`, which also restores the flags the call clobbered.
    trampoline.bytes(&HOOK_BYTES[..STOLEN_LEN]);
    trampoline.absolute_jump(module.resolve_rva(RESUME_RVA)?);

    Ok(trampoline)
}

/// Passes the creature (`r14`) and the address of the `0x66` engine command (`rbp + 0x28`).
const BOOST_ARGUMENT_SETUP: [u8; 7] = [
    0x4c, 0x89, 0xf1, // mov rcx, r14
    0x48, 0x8d, 0x55, 0x28, // lea rdx, [rbp+0x28]
];

fn build_boost_trampoline(module: ProcessModule) -> Result<Trampoline> {
    let mut trampoline = Trampoline::new();

    trampoline.preserved_call(
        on_engine_commands as *const (),
        &BOOST_ARGUMENT_SETUP,
        &[],
        ReturnType::None,
    );
    trampoline.bytes(&BOOST_BYTES[..BOOST_STOLEN_LEN]);
    trampoline.absolute_jump(module.resolve_rva(BOOST_RESUME_RVA)?);

    Ok(trampoline)
}

fn game_time(runtime: &Runtime) -> Option<f64> {
    // SAFETY: the clock is a global of the running game; the read is page-checked anyway.
    unsafe { rd::<f64>(runtime.clock) }.filter(|time| time.is_finite())
}

/// Set from `AiFlaresConfig::allies`: friendly AI ships count as AI too.
static INCLUDE_ALLIES: AtomicBool = AtomicBool::new(false);

fn is_ai(creature: usize) -> bool {
    // Negative and non-zero is the AI side, the same test the missile launch limits use.
    // SAFETY: page-checked reads.
    let Some(side) = (unsafe { rd::<f32>(creature + CREATURE_SIDE) }) else {
        return false;
    };
    if side < 0.0 {
        return true;
    }
    // An allied AI ship: player's side (positive) and not flown by the player (AI flag set; the ship
    // the player flies has it cleared).
    INCLUDE_ALLIES.load(Ordering::Relaxed)
        && side > 0.0
        && unsafe { rd::<u8>(creature + CREATURE_AI_FLAG) }.is_some_and(|flag| flag != 0)
}

fn lock_ships() -> std::sync::MutexGuard<'static, Ships> {
    SHIPS
        .get_or_init(|| Mutex::new(Ships::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn plausible(address: usize) -> bool {
    (0x1_0000..0x7fff_ffff_0000).contains(&address) && address % 8 == 0
}

/// Runs a hook body so that a Rust panic never unwinds into the game (which would abort the process).
fn guarded<T>(name: &str, body: impl FnOnce() -> T) -> Option<T> {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(value) => Some(value),
        Err(_) => {
            if CAUGHT_PANICS.fetch_add(1, Ordering::Relaxed) < 3 {
                log::error!("hf_ai ai_flares: a panic was caught in the {name} hook and ignored");
            }
            None
        }
    }
}

// ---- trace (`log_panic`) -------------------------------------------------------------------------------------

static TRACE: Mutex<Option<File>> = Mutex::new(None);
static TRACE_FAILED: AtomicBool = AtomicBool::new(false);

/// Writes one event line to `Modloader/logs/hf_ai_panic_<unix>.log` (when `log_panic` is on). Never blocks and never
/// panics; called with no other lock held. `t` is the game clock and `ship` the creature pointer, the same values as
/// `t` and `cptr` in `hf_ai_steer_*.csv`, so lines can be joined with the telemetry.
fn trace(runtime: &Runtime, now: f64, creature: usize, event: &str, detail: std::fmt::Arguments) {
    if !runtime.config.log_panic || TRACE_FAILED.load(Ordering::Relaxed) {
        return;
    }
    let Ok(mut guard) = TRACE.try_lock() else {
        return;
    };
    if guard.is_none() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        match File::create(format!("Modloader/logs/hf_ai_panic_{stamp}.log")) {
            Ok(mut file) => {
                let _ = writeln!(
                    file,
                    "# hf_ai v{} ai_flares trace. t = game clock (as in hf_ai_steer_*.csv), ship = creature pointer (cptr there). \
                     START = a panic began (mode flares/evade/panic/roll, side = +1/-1 of the missile's line of approach, boost = engine factor), \
                     EXTEND = a locked-on missile still inside the range, IGNORED = reaction roll failed, END = panic over, \
                     FLIGHT = panic evasion committed to a direction, STEER = waypoint replaced (stock -> new), BOOST/SPIN = engine commands changed, CURTAIN_START/DROP/EXTEND/END = the pre-emptive flare curtain.",
                    env!("CARGO_PKG_VERSION")
                );
                *guard = Some(file);
            }
            Err(error) => {
                TRACE_FAILED.store(true, Ordering::Relaxed);
                log::error!("hf_ai ai_flares: cannot create the panic trace file: {error}");
                return;
            }
        }
    }
    if let Some(file) = guard.as_mut() {
        let _ = file.write_all(format!("{now:.3} 0x{creature:x} {event} {detail}\n").as_bytes());
    }
}

/// Ship position for the trace.
fn ship_position(creature: usize) -> (f32, f32) {
    // SAFETY: page-checked reads.
    unsafe {
        (
            rd(creature + CREATURE_POS_X).unwrap_or(f32::NAN),
            rd(creature + CREATURE_POS_Y).unwrap_or(f32::NAN),
        )
    }
}

/// Warns once, at the first panic, if the flare-crash fix of qol (`enable_flare_crash_fix`) does not look active:
/// more flares in play make the proximity-fuze crash more likely. (Checked late, because the mod that loads second
/// patches after our `init`.)
fn check_flare_crash_fix() {
    if LOGGED_CRASH_FIX_CHECK.swap(true, Ordering::Relaxed) {
        return;
    }
    // SAFETY: page-checked read of the game's code.
    let bytes = unsafe { rd::<[u8; 6]>(crate::IMAGE_BASE + FLARE_CRASH_RVA) };
    if bytes == Some(FLARE_CRASH_ORIGINAL) {
        log::warn!(
            "hf_ai ai_flares: the flare-crash fix does not look installed ({FLARE_CRASH_RVA:#x} has the original bytes). \
             Keep qol `enable_flare_crash_fix` on: AI flares make the proximity-fuze crash more likely"
        );
    } else {
        log::info!("hf_ai ai_flares: flare-crash fix site {FLARE_CRASH_RVA:#x} is patched (good)");
    }
}

/// Ship fields the trace wants after the lock is released.
#[derive(Clone, Copy)]
struct Snapshot {
    roll: bool,
    panic: bool,
    side: f32,
    burst_until: f64,
    evade_until: f64,
    quiet_until: f64,
}

impl Snapshot {
    fn of(ship: &Ship) -> Self {
        Self {
            roll: ship.roll,
            panic: ship.panic,
            side: ship.side,
            burst_until: ship.burst_until,
            evade_until: ship.evade_until,
            quiet_until: ship.quiet_until,
        }
    }

    fn mode(&self, evade: bool) -> &'static str {
        if self.roll {
            "roll"
        } else if self.panic {
            "panic"
        } else if evade {
            "evade"
        } else {
            "flares"
        }
    }
}

// ---- hooks ---------------------------------------------------------------------------------------------------

/// Called from the per-creature update, on the game thread, once per creature per frame.
unsafe extern "C" fn on_creature_update(creature: *mut u8) {
    guarded("creature update", || creature_update(creature as usize));
}

/// For the panic trace: the sides of the panicking ship, the missile's launcher (owner link) and what it is aimed at.
unsafe fn describe_threat(creature: usize, body: usize) -> String {
    let side = |node: usize| if plausible(node) { rd::<f32>(node + CREATURE_SIDE).map_or("?".to_string(), |s| format!("{s:+.0}")) } else { "?".to_string() };
    let owner = rd::<usize>(body + BODY_OWNER_LINK).unwrap_or(0);
    let target = rd::<usize>(body + BODY_TARGET).unwrap_or(0);
    let locked = target != 0 && (target == creature || rd::<usize>(target + BODY_OWNER) == Some(creature));
    let raw = |node: usize| if plausible(node) { rd::<f32>(node + CREATURE_SIDE).map_or("?".to_string(), |s| format!("{s}")) } else { "?".to_string() };
    let class = |node: usize| if plausible(node) { rd::<u8>(node + 0x28).map_or("?".to_string(), |c| format!("{c:#x}")) } else { "?".to_string() };
    format!(
        "ship_side {} launcher_side {} target_side {} (raw {} class {}) locked {} stats {} marker248 {:?} owner {owner:#x} target {target:#x}",
        side(creature),
        side(owner),
        side(target),
        raw(target),
        class(target),
        locked,
        rd::<i32>(body + BODY_STATS_ID).unwrap_or(-1),
        rd::<f32>(body + 0x248)
    )
}

unsafe fn creature_update(creature: usize) {
    let Some(runtime) = RUNTIME.get() else {
        return;
    };
    if !is_ai(creature) {
        return;
    }

    let Some(now) = game_time(runtime) else {
        return;
    };
    let (firing, event, snapshot, ended) = {
        let mut ships = lock_ships();

        if now < ships.last_clock {
            // The game clock restarted: a new battle.
            ships.by_creature.clear();
            ships.curtains.clear();
        }
        ships.last_clock = now;
        if ships.by_creature.len() > PRUNE_THRESHOLD {
            ships.by_creature.retain(|_, ship| !ship.idle(now));
        }

        let ship = ships.by_creature.entry(creature).or_default();
        let mut params = runtime.params;
        params.evade &= crate::maneuvers_allowed(creature);
        let event = panic_math::scan(
            ship,
            now,
            &params,
            || threatened(runtime, creature),
            &mut || rng::random_f32(),
        );
        let ended = ship.trace_active && !ship.panicking(now);
        if matches!(event, ScanEvent::Started(_)) {
            ship.trace_active = true;
        } else if ended {
            ship.trace_active = false;
        }
        (ship.firing(now), event, Snapshot::of(ship), ended)
    };

    if let ScanEvent::Started(_) = event {
        check_flare_crash_fix();
    }
    if runtime.config.log_panic {
        let evade = runtime.config.evade;
        match event {
            ScanEvent::Started(threat) => {
                let ship = ship_position(creature);
                trace(
                    runtime,
                    now,
                    creature,
                    "START",
                    format_args!(
                        "mode={} dist={:.0} missile={:.0},{:.0} ship={:.0},{:.0} side={:+.0} boost={} burst_until={:.3} evade_until={:.3} quiet_until={:.3} | {}",
                        snapshot.mode(evade),
                        threat.distance,
                        threat.position.0,
                        threat.position.1,
                        ship.0,
                        ship.1,
                        snapshot.side,
                        if snapshot.panic || snapshot.roll { runtime.config.panic_boost.max(1.0) } else { 1.0 },
                        snapshot.burst_until,
                        snapshot.evade_until,
                        snapshot.quiet_until,
                        describe_threat(creature, threat.body)
                    ),
                );
            }
            ScanEvent::Seen(threat) => trace(
                runtime,
                now,
                creature,
                "EXTEND",
                format_args!(
                    "dist={:.0} missile={:.0},{:.0} evade_until={:.3} quiet_until={:.3}",
                    threat.distance, threat.position.0, threat.position.1, snapshot.evade_until, snapshot.quiet_until
                ),
            ),
            ScanEvent::Ignored(threat) => trace(
                runtime,
                now,
                creature,
                "IGNORED",
                format_args!(
                    "dist={:.0} missile={:.0},{:.0} reaction_probability={}",
                    threat.distance, threat.position.0, threat.position.1, runtime.config.reaction_probability
                ),
            ),
            ScanEvent::NotDue | ScanEvent::Clear => {}
        }
        if ended {
            trace(runtime, now, creature, "END", format_args!("mode={}", snapshot.mode(evade)));
        }
    }

    if runtime.config.stray_missiles {
        update_tracks(runtime, now);
    }
    curtain_step(runtime, creature, now);

    if firing {
        let (Some(launchers_begin), Some(launchers_end)) = (
            rd::<usize>(creature + CREATURE_LAUNCHERS_BEGIN),
            rd::<usize>(creature + CREATURE_LAUNCHERS_END),
        ) else {
            return;
        };
        if launchers_begin != 0 && launchers_end > launchers_begin {
            release_all(runtime, launchers_begin, launchers_end);
        }
    }
}

/// The flare curtain for one ship: drops flares on a random clock after the player's side launched a missile.
unsafe fn curtain_step(runtime: &Runtime, creature: usize, now: f64) {
    let params = runtime.curtain;
    if params.probability <= 0.0 {
        return;
    }
    // enemy ships react to the player side's launches; allied AI ships (config `allies`) to the enemy's
    let Some(camp) = curtain_camp(creature) else { return };
    let (launch_seq, launch_time) = watch_launches(runtime, now, camp);
    let loaded = flares_loaded(creature);
    let role = curtain_role(runtime, creature, camp);
    let eligible = role != CurtainRole::None;
    let mut params = params;
    if role == CurtainRole::Target {
        params.probability = runtime.config.launch_flares_target_probability;
    }
    let eta = nearest_launched_missile(ship_position(creature), f32::INFINITY, camp)
        .map(|m| { let p = ship_position(creature); (m.0 - p.0).hypot(m.1 - p.1) / runtime.config.launch_flares_missile_speed });
    let (event, index) = {
        let mut ships = lock_ships();
        if ships.curtains.len() > PRUNE_THRESHOLD {
            ships.curtains.retain(|_, c| c.until > 0.0 || c.quiet_until > now);
        }
        let panicking = ships.by_creature.get(&creature).is_some_and(|ship| ship.panicking(now));
        let curtain = ships.curtains.entry(creature).or_default();
        curtain.rack_max = curtain.rack_max.max(loaded);
        curtain.no_flares = curtain.rack_max <= 0.5;
        // no flares at all, or the racks are below the reserve kept for a real panic: retreat only
        let can_drop = loaded > 0.5 && loaded >= runtime.config.launch_flares_min_rack * curtain.rack_max;
        let event = curtain_math::update(curtain, &params, now, launch_seq, launch_time, panicking, can_drop, eligible, eta, &mut || {
            rng::random_f32()
        });
        (event, curtain.drops)
    };
    match event {
        curtain_math::Event::Started => {
            check_flare_crash_fix();
            trace(runtime, now, creature, "CURTAIN_START", format_args!("launch={launch_seq}"));
        }
        curtain_math::Event::Extended => {
            trace(runtime, now, creature, "CURTAIN_EXTEND", format_args!("launch={launch_seq}"));
        }
        curtain_math::Event::Ended => trace(runtime, now, creature, "CURTAIN_END", format_args!("")),
        curtain_math::Event::Drop => {
            trace(runtime, now, creature, "CURTAIN_DROP", format_args!("drop={index} (all launchers)"));
            if let (Some(begin), Some(end)) = (
                rd::<usize>(creature + CREATURE_LAUNCHERS_BEGIN),
                rd::<usize>(creature + CREATURE_LAUNCHERS_END),
            ) {
                if begin != 0 && end > begin {
                    release_all(runtime, begin, end);
                }
            }
        }
        curtain_math::Event::None => {}
    }
}

/// How long one retreat decision keeps the engine boost on (the engine commands follow the steering in the same tick).
const CURTAIN_BOOST_HOLD: f64 = 0.3;

#[derive(Clone, Copy, PartialEq, Eq)]
enum CurtainRole {
    None,
    /// The ship the newest launched missile is aimed at.
    Target,
    /// A ship close to the target.
    Neighbour,
}

/// The role of `creature` for the newest launch: the ship the missile is aimed at, a ship close to it
/// (`launch_flares_neighbor_range`), or nothing. Everybody is a `Neighbour` when `launch_flares_targeted_only` is off.
unsafe fn curtain_role(runtime: &Runtime, creature: usize, camp: usize) -> CurtainRole {
    if !runtime.config.launch_flares_targeted_only {
        return CurtainRole::Neighbour;
    }
    // The link can point at the ship itself or at a part of it (then the part's owner is the ship): take both.
    let (target, owner) = LAUNCHES[camp].try_lock().map(|l| (l.last_target, l.last_target_owner)).unwrap_or((0, 0));
    if target == 0 && owner == 0 {
        return CurtainRole::None;
    }
    if target == creature || owner == creature {
        return CurtainRole::Target;
    }
    let range = runtime.config.launch_flares_neighbor_range;
    if range <= 0.0 {
        return CurtainRole::None;
    }
    let here = ship_position(creature);
    if !(here.0.is_finite() && here.1.is_finite()) {
        return CurtainRole::None;
    }
    // a candidate that is not a ship gives a garbage position: only positions inside the arena's order of magnitude count
    let near = [target, owner].into_iter().filter(|&c| c != 0).any(|c| {
        let p = ship_position(c);
        p.0.is_finite() && p.1.is_finite() && p.0.abs() <= 10_000.0 && p.1.abs() <= 10_000.0 && (here.0 - p.0).hypot(here.1 - p.1) <= range
    });
    if near { CurtainRole::Neighbour } else { CurtainRole::None }
}

/// Flares loaded in all of the ship's launchers (0 = no launcher). Launcher `+0x860` is its count.
unsafe fn flares_loaded(creature: usize) -> f32 {
    let (Some(begin), Some(end)) = (rd::<usize>(creature + CREATURE_LAUNCHERS_BEGIN), rd::<usize>(creature + CREATURE_LAUNCHERS_END))
    else {
        return 0.0;
    };
    if begin == 0 || end <= begin {
        return 0.0;
    }
    launchers(begin, end)
        .filter_map(|launcher| rd::<f32>(launcher + LAUNCHER_COUNT))
        .filter(|count| count.is_finite() && *count > 0.0)
        .sum()
}

/// Missile flight tracks for the stray-missile check: body address -> last position, time and smoothed velocity.
struct Tracks {
    next_scan: f64,
    last_scan: f64,
    map: HashMap<usize, Track>,
}

#[derive(Clone, Copy)]
struct Track {
    pos: (f32, f32),
    vel: (f32, f32),
    time: f64,
}

static TRACKS: Mutex<Option<Tracks>> = Mutex::new(None);

/// Updates the tracks of all live guided missiles at most ten times a second (shared by all ships).
unsafe fn update_tracks(runtime: &Runtime, now: f64) {
    let mut guard = TRACKS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let tracks = guard.get_or_insert_with(|| Tracks { next_scan: 0.0, last_scan: 0.0, map: HashMap::new() });
    if now < tracks.last_scan {
        tracks.map.clear(); // the game clock restarted: a new battle
        tracks.next_scan = 0.0;
    }
    if now < tracks.next_scan {
        return;
    }
    tracks.next_scan = now + panic_math::SCAN_INTERVAL;
    tracks.last_scan = now;
    let mut memory = Region::new();
    let Some(count) = memory.read::<u32>(runtime.body_count) else {
        return;
    };
    let mut fresh: HashMap<usize, Track> = HashMap::new();
    for index in 0..(count as usize).min(MAX_BODIES) {
        let Some(body) = memory.read::<usize>(runtime.bodies + index * 8) else {
            break;
        };
        if body == 0 {
            continue;
        }
        let Some(stats_id) = memory.read::<i32>(body + BODY_STATS_ID) else {
            continue;
        };
        let stats = (runtime.stats)(stats_id);
        if stats.is_null() {
            continue;
        }
        let Some(category) = memory.read::<u32>(stats as usize + STATS_CATEGORY) else {
            continue;
        };
        if category != CATEGORY_TACTICAL_MISSILE && category != CATEGORY_STRATEGIC_LAUNCH {
            continue;
        }
        let (Some(x), Some(y)) = (memory.read::<f32>(body + BODY_POS_X), memory.read::<f32>(body + BODY_POS_Y)) else {
            continue;
        };
        if !(x.is_finite() && y.is_finite()) {
            continue;
        }
        let mut vel = (0.0, 0.0);
        if let Some(previous) = tracks.map.get(&body) {
            let dt = (now - previous.time) as f32;
            if dt > 0.02 && dt < 1.0 {
                let inst = ((x - previous.pos.0) / dt, (y - previous.pos.1) / dt);
                vel = (0.5 * previous.vel.0 + 0.5 * inst.0, 0.5 * previous.vel.1 + 0.5 * inst.1);
                if previous.vel == (0.0, 0.0) {
                    vel = inst;
                }
            }
        }
        fresh.insert(body, Track { pos: (x, y), vel, time: now });
    }
    tracks.map = fresh;
}

/// Snapshot of the tracks (small: a few dozen missiles).
fn track_snapshot() -> HashMap<usize, Track> {
    TRACKS
        .try_lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|t| t.map.clone()))
        .unwrap_or_default()
}

/// Newest launch from the player's side: `(number, game time)`, `(0, 0)` before the first. Scans the bodies at most
/// ten times a second for all ships together.
struct Launches {
    seq: u64,
    time: f64,
    next_scan: f64,
    /// Missile bodies seen alive at the last scan (a body gone from the list is forgotten, so a reused address counts).
    seen: Vec<usize>,
    logged: u32,
    /// Where the player's side's live missiles were at the last scan.
    positions: Vec<(f32, f32)>,
    /// The ship the newest launch is aimed at (0 = unknown).
    last_target: usize,
    /// The target link's owner: the target link can point at a part of the ship, in which case this is the ship.
    last_target_owner: usize,
}

const fn new_launches() -> Mutex<Launches> {
    Mutex::new(Launches { seq: 0, time: 0.0, next_scan: 0.0, seen: Vec::new(), logged: 0, positions: Vec::new(), last_target: 0, last_target_owner: 0 })
}

/// Launch streams by the camp that fired: 0 = the player's side (aimed at an enemy ship), 1 = the enemy (aimed at the
/// player's side: the player or an ally).
static LAUNCHES: [Mutex<Launches>; 2] = [new_launches(), new_launches()];

/// Which launch stream a curtain ship listens to: `None` = it takes no part (the ship the player flies, or allies off).
fn curtain_camp(creature: usize) -> Option<usize> {
    // SAFETY: page-checked reads.
    let side = unsafe { rd::<f32>(creature + CREATURE_SIDE) }?;
    if side < 0.0 {
        return Some(0);
    }
    (side > 0.0 && is_ai(creature)).then_some(1)
}
/// How many detected launches get a diagnostic line (both candidate signals are printed).
const LAUNCH_LOG_LIMIT: u32 = 12;

unsafe fn watch_launches(runtime: &Runtime, now: f64, camp: usize) -> (u64, f64) {
    let mut launches = LAUNCHES[camp].lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if now < launches.time || now + 1.0 < launches.next_scan {
        // the game clock restarted: a new battle
        launches.seen.clear();
        launches.next_scan = 0.0;
        launches.time = 0.0;
    }
    if now < launches.next_scan {
        return (launches.seq, launches.time);
    }
    launches.next_scan = now + panic_math::SCAN_INTERVAL;

    let mut memory = Region::new();
    let Some(count) = memory.read::<u32>(runtime.body_count) else {
        return (launches.seq, launches.time);
    };
    let mut current: Vec<usize> = Vec::new();
    let mut positions: Vec<(f32, f32)> = Vec::new();
    for index in 0..(count as usize).min(MAX_BODIES) {
        let Some(body) = memory.read::<usize>(runtime.bodies + index * 8) else {
            break;
        };
        if body == 0 {
            continue;
        }
        let Some(stats_id) = memory.read::<i32>(body + BODY_STATS_ID) else {
            continue;
        };
        let stats = (runtime.stats)(stats_id);
        if stats.is_null() {
            continue;
        }
        let Some(category) = memory.read::<u32>(stats as usize + STATS_CATEGORY) else {
            continue;
        };
        if category != CATEGORY_TACTICAL_MISSILE && category != CATEGORY_STRATEGIC_LAUNCH {
            continue;
        }
        let Some(vtable) = memory.read::<usize>(body) else {
            continue;
        };
        let Some(is_active_address) = memory.read::<usize>(vtable + VTABLE_IS_ACTIVE) else {
            continue;
        };
        if memory.read::<u8>(is_active_address).is_none() {
            continue;
        }
        let is_active: IsActiveFn = std::mem::transmute(is_active_address);
        if is_active(body as *const u8) == 0 {
            continue;
        }
        let Some(signals) = launch_signals(&mut memory, body) else {
            continue;
        };
        if !signals.fired_by(camp) {
            continue;
        }
        current.push(body);
        if let (Some(x), Some(y)) = (memory.read::<f32>(body + BODY_POS_X), memory.read::<f32>(body + BODY_POS_Y)) {
            if x.is_finite() && y.is_finite() {
                positions.push((x, y));
            }
        }
        if !launches.seen.contains(&body) {
            launches.seq += 1;
            launches.time = now;
            launches.last_target = signals.target_creature;
            launches.last_target_owner = signals.target_owner;
            if launches.logged < LAUNCH_LOG_LIMIT {
                launches.logged += 1;
                log::info!(
                    "hf_ai ai_flares: launch #{} from the {} at t={now:.2}: target side {:?}, target's owner side {:?}, missile's own owner link side {:?} (the owner link is a guess, +0x4b0)",
                    launches.seq,
                    if camp == 0 { "player's side" } else { "enemy" },
                    signals.target_side,
                    signals.target_owner_side,
                    signals.owner_side
                );
            }
        }
    }
    launches.seen = current;
    launches.positions = positions;
    (launches.seq, launches.time)
}

/// What a missile body says about who launched it.
struct LaunchSignals {
    /// Side of the thing the missile's target link points at, if it looks like a ship.
    target_side: Option<f32>,
    /// Side of that thing's owner (the link can point at a part of a ship).
    target_owner_side: Option<f32>,
    /// Side of the missile's own owner link (inferred from `FUN_14002ff60`, only logged).
    owner_side: Option<f32>,
    /// The ship the missile is aimed at: the target link itself or its owner, whichever reads as a ship (0 = unknown).
    target_creature: usize,
    /// The target link's owner pointer (0 = none).
    target_owner: usize,
}

impl LaunchSignals {
    /// The proven signal: the missile aims at an enemy ship, so the player's side launched it (the game launches only
    /// at the owner's current target). The owner link is not used until the log confirms it.
    fn fired_by(&self, camp: usize) -> bool {
        // A target link can point at a ship or at a part of one, so both readings are taken; if they disagree
        // (junk read as a side) the launch is not counted rather than guessed.
        let mut sides = [self.target_side, self.target_owner_side].into_iter().flatten().peekable();
        sides.peek().is_some() && sides.all(|side| if camp == 0 { side < 0.0 } else { side > 0.0 })
    }
}

/// A side value that looks like one: finite, non-zero, small.
fn valid_side(side: f32) -> Option<f32> {
    (side.is_finite() && side != 0.0 && side.abs() <= 100.0).then_some(side)
}

/// Missile owner link node (the link at `+0x4a8`, node pointer 8 bytes in like the target link at `+0x490/+0x498`).
const BODY_OWNER_LINK: usize = 0x4b0;

unsafe fn launch_signals(memory: &mut Region, body: usize) -> Option<LaunchSignals> {
    let target = memory.read::<usize>(body + BODY_TARGET).filter(|&t| t != 0 && plausible(t))?;
    let side_at = |memory: &mut Region, node: usize| {
        if node == 0 || !plausible(node) {
            return None;
        }
        memory.read::<f32>(node + CREATURE_SIDE).and_then(valid_side)
    };
    let target_side = side_at(memory, target);
    let target_owner = memory.read::<usize>(target + BODY_OWNER).unwrap_or(0);
    let target_owner_side = side_at(memory, target_owner);
    let owner = memory.read::<usize>(body + BODY_OWNER_LINK).unwrap_or(0);
    let owner_side = side_at(memory, owner);
    let target_creature = if target_side.is_some() {
        target
    } else if target_owner_side.is_some() {
        target_owner
    } else {
        0
    };
    Some(LaunchSignals { target_side, target_owner_side, owner_side, target_creature, target_owner })
}

/// Whether `creature` is in a missile panic right now (flares or evasion). Read-only; `true` when the table is busy
/// (the caller then leaves the ship to the panic stage, which keeps the waypoint it sees identical to v0.2.0).
pub fn panicking(creature: usize, now: f64) -> bool {
    if RUNTIME.get().is_none() {
        return false;
    }
    let Some(ships) = SHIPS.get() else { return false };
    match ships.try_lock() {
        Ok(guard) => guard.by_creature.get(&creature).is_some_and(|s| s.panicking(now)),
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner().by_creature.get(&creature).is_some_and(|s| s.panicking(now)),
        Err(std::sync::TryLockError::WouldBlock) => true,
    }
}

/// The panic stage of the steering hook, called by `lib.rs` after the waypoint stage (`decide`) with the waypoint
/// the AI tick would store (`wanted`, possibly already changed by `decide`) and the ship position. Returns the
/// replacement waypoint while the ship is evading, `None` to leave it. Never panics.
pub unsafe fn steer(creature: usize, now: f64, wanted: (f32, f32), ship: (f32, f32)) -> Option<(f32, f32)> {
    guarded("steering", || steering(creature, now, wanted, ship)).flatten()
}

unsafe fn steering(creature: usize, now: f64, wanted: (f32, f32), ship: (f32, f32)) -> Option<(f32, f32)> {
    let runtime = RUNTIME.get()?;
    // hf_ai: with evade off the original installs no steering hook at all; this keeps that explicit.
    if !runtime.params.evade || !is_ai(creature) || !crate::maneuvers_allowed(creature) {
        return None;
    }

    let (outcome, snapshot, trace_steer) = {
        let mut guard = lock_ships();
        let ships = &mut *guard;
        let state = ships.by_creature.get_mut(&creature)?;
        let mut outcome = panic_math::steer(state, now, &runtime.params, wanted, ship);
        if outcome.waypoint.is_none() && !state.panicking(now) && runtime.config.launch_flares_evade > 0.0 {
            // the curtain runs: get away from the launched missile instead of flying into it
            let curtain = ships.curtains.get(&creature).filter(|c| c.until > now).map(|c| c.side);
            if let Some(side) = curtain {
                let no_flares = ships.curtains.get(&creature).is_some_and(|c| c.no_flares || !c.drops_enabled);
                outcome.waypoint = nearest_launched_missile(ship, runtime.config.launch_flares_evade_range, curtain_camp(creature).unwrap_or(0)).and_then(|missile| {
                    // a ship without flares shelters behind a dropping ally; otherwise (or without one) it runs from the missile
                    let shelter = if no_flares && runtime.config.launch_flares_shelter {
                        nearest_dropper(&ships.curtains, creature, ship, now, runtime.config.launch_flares_shelter_range)
                            .and_then(|dropper| curtain_math::shelter_waypoint(wanted, ship, dropper, missile, runtime.config.launch_flares_evade))
                    } else {
                        None
                    };
                    shelter.or_else(|| curtain_math::away_waypoint(wanted, ship, missile, side, runtime.config.launch_flares_evade))
                });
                outcome.waypoint = outcome.waypoint.map(|wp| crate::tactics::clamp_to_arena(wp, ship));
                if outcome.waypoint.is_some() {
                    if let Some(c) = ships.curtains.get_mut(&creature) {
                        c.boost_until = now + CURTAIN_BOOST_HOLD;
                    }
                }
            }
        }
        let trace_steer = runtime.config.log_panic
            && outcome.waypoint.is_some()
            && now - state.trace_last_steer >= TRACE_PERIOD;
        if trace_steer {
            state.trace_last_steer = now;
        }
        (outcome, Snapshot::of(state), trace_steer)
    };

    if runtime.config.log_panic {
        if let (true, Some((x, y))) = (outcome.new_flight, outcome.waypoint) {
            let length = (x - ship.0).hypot(y - ship.1).max(1.0e-4);
            trace(
                runtime,
                now,
                creature,
                "FLIGHT",
                format_args!("side={:+.0} dir={:.3},{:.3}", snapshot.side, (x - ship.0) / length, (y - ship.1) / length),
            );
        }
        if let (true, Some((x, y))) = (trace_steer, outcome.waypoint) {
            trace(
                runtime,
                now,
                creature,
                "STEER",
                format_args!(
                    "mode={} side={:+.0} ship={:.0},{:.0} stock={:.0},{:.0} new={:.0},{:.0}",
                    snapshot.mode(true),
                    snapshot.side,
                    ship.0,
                    ship.1,
                    wanted.0,
                    wanted.1,
                    x,
                    y
                ),
            );
        }
    }
    outcome.waypoint
}

/// Called from the AI tick just before it sends the engine commands out. During a panic evasion
/// raises the strength of the `0x66` and `0x67` commands to the configured boost, the way the
/// player's Shift key does.
unsafe extern "C" fn on_engine_commands(creature: *mut u8, command: *mut f32) {
    guarded("engine commands", || engine_commands(creature as usize, command));
}

unsafe fn engine_commands(creature: usize, command: *mut f32) {
    let Some(runtime) = RUNTIME.get() else {
        return;
    };
    if command.is_null() || !is_ai(creature) || !crate::maneuvers_allowed(creature) {
        return;
    }

    let Some(now) = game_time(runtime) else {
        return;
    };
    let mode = {
        let mut ships = lock_ships();
        ships.by_creature.get_mut(&creature).and_then(|ship| {
            let (roll, side) = panic_math::engine_mode(ship, now)?;
            let trace_engine = runtime.config.log_panic && now - ship.trace_last_engine >= TRACE_PERIOD;
            if trace_engine {
                ship.trace_last_engine = now;
            }
            Some((roll, side, trace_engine))
        })
    };
    let Some((roll, side, trace_engine)) = mode else {
        dive_boost(creature, command, now);
        return;
    };

    let boost = runtime.config.panic_boost.max(1.0);
    if roll {
        let angles = spin(runtime, creature, command, side, boost);
        if let (true, Some((heading, first, second))) = (trace_engine, angles) {
            trace(
                runtime,
                now,
                creature,
                "SPIN",
                format_args!("side={side:+.0} boost={boost} heading={heading:.3} angles={first:.3},{second:.3}"),
            );
        }
        return;
    }
    let mut changed = [(0.0f32, 0.0f32); 2];
    for (slot, offset) in [COMMAND_STRENGTH, COMMAND_NEXT_STRENGTH].into_iter().enumerate() {
        let strength = command.byte_add(offset);
        let current = strength.read_unaligned();
        if let Some(boosted) = panic_math::boosted_strength(current, boost) {
            strength.write_unaligned(boosted);
            changed[slot] = (current, boosted);
        }
    }
    if trace_engine {
        trace(
            runtime,
            now,
            creature,
            "BOOST",
            format_args!(
                "boost={boost} strength0={:.2}->{:.2} strength1={:.2}->{:.2}",
                changed[0].0, changed[0].1, changed[1].0, changed[1].1
            ),
        );
    }
}

/// The tactics' "deny the dive" boost: the same strength raise as the panic boost, no roll.
unsafe fn dive_boost(creature: usize, command: *mut f32, now: f64) {
    let mut boost = crate::tactics::dive_boost_for(creature, now);
    if let Some(runtime) = RUNTIME.get() {
        if runtime.config.launch_flares_boost > 1.0
            && lock_ships().curtains.get(&creature).is_some_and(|c| c.until > now && c.boost_until >= now)
        {
            boost = boost.max(runtime.config.launch_flares_boost);
        }
    }
    if boost <= 1.0 {
        return;
    }
    for offset in [COMMAND_STRENGTH, COMMAND_NEXT_STRENGTH] {
        let strength = command.byte_add(offset);
        if let Some(boosted) = panic_math::boosted_strength(strength.read_unaligned(), boost) {
            strength.write_unaligned(boosted);
        }
    }
}

/// What the player's Q or E with Shift does: the two engine groups (the `0x66`/`0x64` and the
/// `0x67`/`0x65` commands) are told to thrust in opposite directions, perpendicular to the ship's
/// heading, so one side of the ship pushes up and the other down and it spins on the spot. The
/// command angle of a direction `(x, y)` is `atan2(x, -y)` (`vectorCalculateAngles_`), which for
/// the perpendicular `(sin h, -cos h)` of heading `h` is `h` itself.
/// Returns `(heading, first, second)` when the commands were rewritten.
unsafe fn spin(
    runtime: &Runtime,
    creature: usize,
    command: *mut f32,
    side: f32,
    boost: f32,
) -> Option<(f32, f32, f32)> {
    let body = (runtime.creature_body)(creature as *const u8);
    if body == 0 || !plausible(body) {
        return None;
    }
    let heading: f32 = rd(body + BODY_HEADING)?;
    let (first, second) = panic_math::spin_angles(heading, side)?;

    for (offset, angle) in [
        (COMMAND_ANGLE as isize, first),
        (COMMAND_NEXT_ANGLE as isize, second),
        (COMMAND_PREV2_ANGLE, first),
        (COMMAND_PREV_ANGLE, second),
    ] {
        command.byte_offset(offset).write_unaligned(angle);
    }
    for offset in [COMMAND_STRENGTH, COMMAND_NEXT_STRENGTH] {
        command.byte_add(offset).write_unaligned(boost);
    }
    Some((heading, first, second))
}

fn launchers(begin: usize, end: usize) -> impl Iterator<Item = usize> {
    (begin..end)
        .step_by(LAUNCHER_ENTRY_SIZE)
        .take(MAX_LAUNCHERS)
        .filter_map(|entry| {
            // SAFETY: page-checked reads; the vector belongs to a live creature that the game is updating.
            let launcher: usize = unsafe { rd(entry + LAUNCHER_PTR) }?;
            (plausible(launcher) && unsafe { readable(launcher) }).then_some(launcher)
        })
}

/// The nearest other ship within `range` whose curtain is running and dropping flares: shelter for a ship without flares.
fn nearest_dropper(
    curtains: &HashMap<usize, Curtain>,
    me: usize,
    ship: (f32, f32),
    now: f64,
    range: f32,
) -> Option<(f32, f32)> {
    curtains
        .iter()
        .filter(|(&c, curtain)| c != me && curtain.until > now && curtain.drops_enabled && curtain.drops > 0)
        .map(|(&c, _)| ship_position(c))
        .filter(|p| p.0.is_finite() && p.1.is_finite() && p.0.abs() <= 10_000.0 && p.1.abs() <= 10_000.0)
        .map(|p| ((p.0 - ship.0).hypot(p.1 - ship.1), p))
        .filter(|(d, _)| *d <= range)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, p)| p)
}

/// The nearest live missile of the player's side to `ship` within `range`, from the last launch scan.
fn nearest_launched_missile(ship: (f32, f32), range: f32, camp: usize) -> Option<(f32, f32)> {
    let launches = LAUNCHES[camp].try_lock().ok()?;
    launches
        .positions
        .iter()
        .map(|&m| ((m.0 - ship.0).hypot(m.1 - ship.1), m))
        .filter(|(d, _)| *d <= range)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, m)| m)
}

unsafe fn release_all(runtime: &Runtime, begin: usize, end: usize) {
    for launcher in launchers(begin, end) {
        (runtime.release)(launcher as *mut u8);
    }
}

/// True when the missile `body` was launched by the camp of `creature` (so it is not a threat to it). The launcher's side
/// (owner link, which the launch log showed to be right for both camps) decides; without it, a missile aimed at the other camp.
unsafe fn friendly_stray(memory: &mut Region, creature: usize, body: usize, target: usize) -> bool {
    let side_of = |memory: &mut Region, node: usize| {
        // only a ship (creature class bits 0x2f at +0x28) has a side at +0xad4; a plane or missile body gives garbage there
        if node == 0 || !plausible(node) || memory.read::<u8>(node + 0x28).map_or(true, |class| class & 0x2f != 0x2f) {
            return None;
        }
        memory.read::<f32>(node + CREATURE_SIDE).and_then(valid_side)
    };
    let Some(own) = side_of(memory, creature) else { return false };
    // aimed at something that is not a ship (a plane: the K-13; another missile): it is not coming for this ship, whoever
    // launched it (decided first: the log showed a player-side K-13 at a plane passing an enemy ship)
    if target != 0 && plausible(target) && memory.read::<u8>(target + 0x28).is_some_and(|class| class & 0x2f != 0x2f) {
        return true;
    }
    let owner = memory.read::<usize>(body + BODY_OWNER_LINK).unwrap_or(0);
    if let Some(launcher) = side_of(memory, owner) {
        return launcher.signum() == own.signum();
    }
    // the K-13 (carrier planes' air-to-air missile) stores its owner's side x 100 at +0x248, no usable owner link
    if let Some(marker) = memory.read::<f32>(body + 0x248).filter(|m| m.is_finite() && *m != 0.0 && m.abs() <= 10_000.0) {
        return marker.signum() == own.signum();
    }
    let aimed = side_of(memory, target).or_else(|| {
        let target_owner = memory.read::<usize>(target + BODY_OWNER).unwrap_or(0);
        side_of(memory, target_owner)
    });
    match aimed {
        Some(aimed) => aimed.signum() != own.signum(),
        // aimed at something that is not a ship (a plane, another missile): not coming for this ship
        None => target != 0 && plausible(target),
    }
}

/// The nearest live missile locked on to `creature` that is closer than the trigger range, if there is one.
/// Same scan as the original; every read is page-checked through a small region cache ([`Region`]), the body
/// count is capped, and the `is_active` function pointer must point into readable memory before it is called.
unsafe fn threatened(runtime: &Runtime, creature: usize) -> Option<Threat> {
    let mut memory = Region::new();
    let count: u32 = memory.read(runtime.body_count)?;
    let range = runtime.config.trigger_range;
    let mut nearest: Option<Threat> = None;
    let stray_enabled = runtime.config.stray_missiles && crate::maneuvers_allowed(creature);
    let tracks = if stray_enabled { track_snapshot() } else { HashMap::new() };
    let here = if stray_enabled { ship_position(creature) } else { (f32::NAN, f32::NAN) };
    let size = if stray_enabled { memory.read::<f32>(creature + CREATURE_RADIUS).unwrap_or(0.0) } else { 0.0 };

    for index in 0..(count as usize).min(MAX_BODIES) {
        let Some(body) = memory.read::<usize>(runtime.bodies + index * 8) else {
            break;
        };
        if body == 0 {
            continue;
        }

        let Some(stats_id) = memory.read::<i32>(body + BODY_STATS_ID) else {
            continue;
        };
        let stats = (runtime.stats)(stats_id);
        if stats.is_null() {
            continue;
        }
        let Some(category) = memory.read::<u32>(stats as usize + STATS_CATEGORY) else {
            continue;
        };
        if category != CATEGORY_TACTICAL_MISSILE && category != CATEGORY_STRATEGIC_LAUNCH {
            continue;
        }

        let target = memory.read::<usize>(body + BODY_TARGET).unwrap_or(0);
        let direct = target == creature;
        let locked = target != 0 && (direct || memory.read::<usize>(target + BODY_OWNER) == Some(creature));
        let mut stray = false;
        if !locked {
            // not locked on this ship: only a stray whose flight path crosses the ship counts (never its own missiles)
            if !stray_enabled || memory.read::<usize>(body + BODY_OWNER_LINK) == Some(creature) {
                continue;
            }
            // a missile of this ship's own camp is no threat even when its path crosses the ship (an enemy Zenith aimed at
            // the player must not make the other enemies panic); the launcher's side first, the aimed-at side if unknown
            if friendly_stray(&mut memory, creature, body, target) {
                continue;
            }
            stray = true;
        }

        let Some(vtable) = memory.read::<usize>(body) else {
            continue;
        };
        let Some(is_active_address) = memory.read::<usize>(vtable + VTABLE_IS_ACTIVE) else {
            continue;
        };
        if memory.read::<u8>(is_active_address).is_none() {
            continue;
        }
        let is_active: IsActiveFn = std::mem::transmute(is_active_address);
        if is_active(body as *const u8) == 0 {
            continue;
        }

        let distance = (runtime.distance)(creature as *const u8, body as *const u8);
        let limit = if stray { runtime.config.stray_missile_range } else { range };
        if stray {
            let in_the_way = tracks.get(&body).is_some_and(|track| {
                here.0.is_finite()
                    && here.1.is_finite()
                    && panic_math::stray_cpa(
                        here,
                        track.pos,
                        track.vel,
                        runtime.config.stray_missile_radius + 0.5 * size.clamp(0.0, 600.0),
                        runtime.config.stray_missile_eta_s,
                    )
                    .is_some()
            });
            if !in_the_way {
                continue;
            }
        }
        if distance <= limit && nearest.map_or(true, |best| distance < best.distance) {
            let (Some(x), Some(y)) = (
                memory.read::<f32>(body + BODY_POS_X),
                memory.read::<f32>(body + BODY_POS_Y),
            ) else {
                continue;
            };
            nearest = Some(Threat { distance, position: (x, y), body, stray });

            if stray && !LOGGED_STRAY.swap(true, Ordering::Relaxed) {
                log::info!("hf_ai ai_flares: first STRAY missile threat (not locked on this ship, its path crosses it): distance {distance:.0}, missile at {x:.0},{y:.0}");
            }
            let logged = if direct {
                &LOGGED_DIRECT_TARGET
            } else {
                &LOGGED_CHILD_TARGET
            };
            if !logged.swap(true, Ordering::Relaxed) {
                log::info!(
                    "hf_ai ai_flares: first threat, missile link points at the {} (distance {distance:.0}, category {category:#x}, missile at {x:.0},{y:.0})",
                    if direct { "ship itself" } else { "a part of the ship" },
                );
            }
        }
    }

    nearest
}
