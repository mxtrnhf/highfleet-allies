//! Lets AI ships panic against incoming missiles: dump flares and evade (Steam 1.163 only).
//!
//! **hf_ai port** of `hf_qol_fork/src/ai_flares.rs` (fork branch `ai-flares`, commit 5c3ab73). The port started
//! behaviour-identical; this version adds stray-missile threats, single-burst handling, per-missile reaction rolls
//! and the hf_ally debug feed.
//! The text below is the original description, kept mostly as is. What changed in this port: the steering hook at
//! `0x140064a8d` and its scratch buffer belong to `lib.rs` (always installed there, log-only without ai_flares), which
//! calls [`panic_waypoint`] after the waypoint stage `decide()`; the timers and direction math live in `panic_math.rs`
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
//!    creature and closer than `trigger_range`; a missile rolls `reaction_probability` once
//!    when it first appears, and on success starts a burst of `burst_seconds`, followed by
//!    `min_interval_seconds` of quiet. A failed roll ignores that missile body until the
//!    threat picture clears.
//!
//! While a panic runs the ship also evades. The AI tick (`0x140062f90`) ends its steering by
//! storing a desired waypoint, 200 units from the ship along the wanted direction (the tactic
//! override uses 400, see `tactics_math::WAYPOINT_LEN`), at
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
//! commands on its stack (`rbp + 0x28` and `rbp + 0x48`, 0x20 bytes apart, strength at `+4`), so a
//! third hook at `0x140065744`, just before they are sent to the engines, can rewrite them.
//!
//! Raising the strength is not enough to make an AI ship *run*: the tick's force law is a velocity
//! controller whose target speed is `clamp(sqrt(distance to waypoint + 0.1) / 20, 0, 1) * 25 * TWR`
//! (`0x140064b2a..0x140064b7e`), and it brakes as soon as the ship reaches it. Every boost therefore
//! also carries the direction the manoeuvre steers along (the tactic waypoint, or the committed
//! panic flight direction), and both banks' `0x66`/`0x67` angles and `0x64`/`0x65` servo angles are
//! pointed there with the strength floored at the boost: the stock controller is bypassed and the
//! ship dashes like a player holding Shift.
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
//! | creature: side float / launcher vector       | `+0xad4` (negative = enemy, positive = player's side) / `+0x1408..0x1410`, 0x18-byte entries, launcher at `+8` |
//! | launcher: cooldown / count / rate timer      | `+0x848` / `+0x860` / `+0x86c` (floats)  |
//! | body: stats id / target link pointer         | `+0x188` / `+0x498`; link owner `+0xb8`  |
//! | body: position x, y                          | `+0x128`, `+0x12c` (floats)              |

use std::{
    collections::HashMap,
    ffi::c_void,
    fs::File,
    io::Write as _,
    mem::offset_of,
    panic::{catch_unwind, AssertUnwindSafe},
    ptr,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Mutex, OnceLock,
    },
};

use patchy::{PatchSession, ProcessModule, Result, ReturnType, Trampoline};
use windows_sys::Win32::System::Memory::{VirtualQuery, MEMORY_BASIC_INFORMATION};

use crate::native_layout::{self, Link};
use crate::{
    flares_config::AiFlaresConfig,
    panic_math::{self, PanicParams, PanicState, Threat, ThreatEvent, ThreatScan},
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
const BODY_HEADING: usize = native_layout::BODY_HEADING;
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

const CREATURE_SIDE: usize = native_layout::CREATURE_SIDE;
/// Offset in a ship of the AI flag: non-zero when the ship is not flown by the player.
const CREATURE_AI_FLAG: usize = native_layout::CREATURE_AI_FLAG;
const CREATURE_LAUNCHERS_BEGIN: usize = native_layout::CREATURE_FLARE_LAUNCHERS;
const CREATURE_LAUNCHERS_END: usize = CREATURE_LAUNCHERS_BEGIN + std::mem::size_of::<usize>();
const LAUNCHER_ENTRY_SIZE: usize = native_layout::LINK_STRIDE;
const LAUNCHER_PTR: usize = offset_of!(Link, target);
const BODY_POS_X: usize = native_layout::BODY_POS_X;
const BODY_POS_Y: usize = native_layout::BODY_POS_Y;
const BODY_STATS_ID: usize = native_layout::BODY_PART_INDEX;
const BODY_TARGET: usize = native_layout::BODY_PROJECTILE_TARGET;
const NODE_OWNER: usize = native_layout::NODE_OWNER;
const STATS_CATEGORY: usize = native_layout::STATS_CATEGORY;
const VTABLE_IS_ACTIVE: usize = 0x78;

/// Stats categories of guided missiles (tactical and strategic).
const CATEGORY_TACTICAL_MISSILE: u32 = 0x4000;
const CATEGORY_STRATEGIC_MISSILE: u32 = 0x800_0000;

/// Per-creature state is dropped once the table grows past this and the entries are idle.
const PRUNE_THRESHOLD: usize = 128;
/// hf_ai hardening (the original loops are unbounded): most bodies / launchers a scan will look at.
const MAX_BODIES: usize = 65_536;
const MAX_LAUNCHERS: usize = 256;
/// Seconds between two trace lines of the same kind for one ship (steering and engine hooks run every frame).
const TRACE_PERIOD: f64 = 0.5;

/// Creature bounding-box centre (the position the AI uses), for the trace.
/// Ship bounding radius (`Creature::bounding_radius`).
const CREATURE_RADIUS: usize = native_layout::CREATURE_RADIUS;
const CREATURE_POS_X: usize = native_layout::CREATURE_POS_X;
const CREATURE_POS_Y: usize = native_layout::CREATURE_POS_Y;
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
    params: PanicParams,
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
    by_creature: HashMap<usize, PanicState>,
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

/// Whether missile defense requests the shared engine hook, independently of flare-site validation.
pub fn wants_engine_boost(config: &AiFlaresConfig) -> bool {
    let config = config.sanitized();
    config.enabled
        && config.evade
        && ((config.panic_boost > 1.0 && config.panic_evasion_probability > 0.0)
            || config.barrel_roll_probability > 0.0)
}

/// Prepares the flare site; the caller prepares the shared boost and steering sites independently.
/// `steering_hook_ok` says whether `lib.rs` found the steering code as expected and prepared its hook.
/// A site whose game code is not what was analysed is skipped, with the reason in the log.
pub unsafe fn patch_ai_flares(
    session: &mut PatchSession,
    config: &AiFlaresConfig,
    steering_hook_ok: bool,
    boost: bool,
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

    if !boost {
        config.panic_boost = 1.0;
    }

    let runtime = Runtime {
        config: config.clone(),
        params: PanicParams {
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
    Ok(())
}

/// Installs the engine-command boost detour at `0x140065744` when either `ai_flares` or `tactics` needs engine boost.
pub unsafe fn prepare_boost_hook(
    session: &mut PatchSession,
    module: ProcessModule,
    steering_hook_ok: bool,
) -> Result<bool> {
    let boost_hook_ok = steering_hook_ok
        && code_matches(module, BOOST_RVA, &BOOST_BYTES)?
        && code_matches(module, BOOST_COMMAND_RVA, &BOOST_COMMAND_BYTES)?;
    if !boost_hook_ok {
        log::error!(
            "hf_ai: the engine command code at {BOOST_RVA:#x} has other bytes ({OWNERSHIP_HINT}), engine boost stays off"
        );
        return Ok(false);
    }
    session.detour_trampoline(
        module.resolve_rva(BOOST_RVA)?,
        BOOST_STOLEN_LEN,
        build_boost_trampoline(module)?,
    )?;
    Ok(true)
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
/// panics; called with no other lock held. `t` is the game clock and `ship` the creature pointer.
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
                    "# hf_ai v{} ai_flares trace. t = game clock, ship = creature pointer. \
                     START = a panic began (mode flares/evade/panic/roll, side = +1/-1 of the missile's line of approach, boost = engine factor), \
                     EXTEND = a locked-on missile still inside the range, IGNORED = reaction roll failed, END = panic over, \
                     FLIGHT = panic evasion committed to a direction, STEER = waypoint replaced (stock -> new), BOOST/SPIN = engine commands changed, ESCAPE = both banks aimed along the escape direction at the boost.",
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
    barrel_roll: bool,
    panic_evasion: bool,
    dodge_side: f32,
    flare_burst_until: f64,
    maneuver_until: f64,
    cooldown_until: f64,
}

impl Snapshot {
    fn of(ship: &PanicState) -> Self {
        Self {
            barrel_roll: ship.barrel_roll,
            panic_evasion: ship.panic_evasion,
            dodge_side: ship.dodge_side,
            flare_burst_until: ship.flare_burst_until,
            maneuver_until: ship.maneuver_until,
            cooldown_until: ship.cooldown_until,
        }
    }

    fn mode(&self, evade: bool) -> &'static str {
        if self.barrel_roll {
            "roll"
        } else if self.panic_evasion {
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
    let owner = rd::<usize>(body + MISSILE_OWNER_LINK).unwrap_or(0);
    let target = rd::<usize>(body + BODY_TARGET).unwrap_or(0);
    let locked = target != 0 && (target == creature || rd::<usize>(target + NODE_OWNER) == Some(creature));
    let raw = |node: usize| if plausible(node) { rd::<f32>(node + CREATURE_SIDE).map_or("?".to_string(), |s| format!("{s}")) } else { "?".to_string() };
    let class = |node: usize| if plausible(node) { rd::<u8>(node + native_layout::NODE_CODE).map_or("?".to_string(), |c| format!("{c:#x}")) } else { "?".to_string() };
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
            crate::debug_state::clear();
            ships.by_creature.clear();
        }
        ships.last_clock = now;
        if ships.by_creature.len() > PRUNE_THRESHOLD {
            ships.by_creature.retain(|_, ship| !ship.is_idle(now));
        }

        let ship = ships.by_creature.entry(creature).or_default();
        let mut params = runtime.params;
        params.evade &= crate::maneuvers_allowed(creature);
        let event = panic_math::scan(
            ship,
            now,
            &params,
            |ignored| threatened(runtime, creature, ignored),
            &mut || rng::random_f32(),
        );
        let ended = ship.trace_active && !ship.is_panicking(now);
        if matches!(event, ThreatEvent::PanicStarted(_)) {
            ship.trace_active = true;
        } else if ended {
            ship.trace_active = false;
        }
        (ship.releasing_flares(now), event, Snapshot::of(ship), ended)
    };

    let flare_code = if now < snapshot.maneuver_until {
        if snapshot.barrel_roll {
            crate::debug_state::FLARES_ROLL
        } else if snapshot.panic_evasion {
            crate::debug_state::FLARES_PANIC
        } else {
            crate::debug_state::FLARES_EVADE
        }
    } else if now < snapshot.flare_burst_until {
        crate::debug_state::FLARES_BURST
    } else {
        crate::debug_state::FLARES_NONE
    };
    crate::debug_state::set_flares(creature, now, flare_code);

    if let ThreatEvent::PanicStarted(_) = event {
        check_flare_crash_fix();
    }
    if runtime.config.log_panic {
        let evade = runtime.config.evade;
        match event {
            ThreatEvent::PanicStarted(threat) => {
                let ship = ship_position(creature);
                trace(
                    runtime,
                    now,
                    creature,
                    "START",
                    format_args!(
                        "mode={} dist={:.0} missile={:.0},{:.0} ship={:.0},{:.0} side={:+.0} boost={} flare_burst_until={:.3} maneuver_until={:.3} cooldown_until={:.3} | {}",
                        snapshot.mode(evade),
                        threat.distance,
                        threat.position.0,
                        threat.position.1,
                        ship.0,
                        ship.1,
                        snapshot.dodge_side,
                        if snapshot.panic_evasion || snapshot.barrel_roll { runtime.config.panic_boost.max(1.0) } else { 1.0 },
                        snapshot.flare_burst_until,
                        snapshot.maneuver_until,
                        snapshot.cooldown_until,
                        describe_threat(creature, threat.body)
                    ),
                );
            }
            ThreatEvent::ThreatContinues(threat) => trace(
                runtime,
                now,
                creature,
                "EXTEND",
                format_args!(
                    "dist={:.0} missile={:.0},{:.0} maneuver_until={:.3} cooldown_until={:.3}",
                    threat.distance, threat.position.0, threat.position.1, snapshot.maneuver_until, snapshot.cooldown_until
                ),
            ),
            ThreatEvent::ReactionRollFailed(threat) => trace(
                runtime,
                now,
                creature,
                "IGNORED",
                format_args!(
                    "dist={:.0} missile={:.0},{:.0} reaction_probability={}",
                    threat.distance, threat.position.0, threat.position.1, runtime.config.reaction_probability
                ),
            ),
            ThreatEvent::NotDue | ThreatEvent::NoThreat => {}
        }
        if ended {
            trace(runtime, now, creature, "END", format_args!("mode={}", snapshot.mode(evade)));
        }
    }

    if runtime.config.stray_missiles {
        update_tracks(runtime, now);
    }

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
        if category != CATEGORY_TACTICAL_MISSILE && category != CATEGORY_STRATEGIC_MISSILE {
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

/// A side value that looks like one: finite, non-zero, small.
fn valid_side(side: f32) -> Option<f32> {
    (side.is_finite() && side != 0.0 && side.abs() <= 100.0).then_some(side)
}

/// Missile owner link node (the `Body::projectile_link_2` target).
const MISSILE_OWNER_LINK: usize = native_layout::BODY_PROJECTILE_OWNER;

/// Whether `creature` is in a missile panic right now (flares or evasion). Read-only; `true` when the table is busy
/// (the caller then leaves the ship to the panic stage, which keeps the waypoint it sees identical to v0.2.0).
pub fn panicking(creature: usize, now: f64) -> bool {
    if RUNTIME.get().is_none() {
        return false;
    }
    let Some(ships) = SHIPS.get() else { return false };
    match ships.try_lock() {
        Ok(guard) => guard.by_creature.get(&creature).is_some_and(|s| s.is_panicking(now)),
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner().by_creature.get(&creature).is_some_and(|s| s.is_panicking(now)),
        Err(std::sync::TryLockError::WouldBlock) => true,
    }
}

/// The panic stage of the steering hook, called by `lib.rs` after the waypoint stage (`decide`) with the waypoint
/// the AI tick would store (`wanted`, possibly already changed by `decide`) and the ship position. Returns the
/// replacement waypoint while the ship is evading, `None` to leave it. Never panics.
pub unsafe fn panic_waypoint(creature: usize, now: f64, wanted: (f32, f32), ship: (f32, f32)) -> Option<(f32, f32)> {
    guarded("panic waypoint", || panic_waypoint_inner(creature, now, wanted, ship)).flatten()
}

unsafe fn panic_waypoint_inner(creature: usize, now: f64, wanted: (f32, f32), ship: (f32, f32)) -> Option<(f32, f32)> {
    let runtime = RUNTIME.get()?;
    // hf_ai: with evade off the original installs no steering hook at all; this keeps that explicit.
    if !runtime.params.evade || !is_ai(creature) || !crate::maneuvers_allowed(creature) {
        return None;
    }

    let (outcome, snapshot, trace_steer) = {
        let mut guard = lock_ships();
        let ships = &mut *guard;
        let state = ships.by_creature.get_mut(&creature)?;
        let outcome = panic_math::evasive_waypoint(state, now, &runtime.params, wanted, ship);
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
                format_args!("side={:+.0} dir={:.3},{:.3}", snapshot.dodge_side, (x - ship.0) / length, (y - ship.1) / length),
            );
        }
        if let (true, Some((x, y))) = (trace_steer, outcome.waypoint) {
            trace(
                runtime,
                now,
                creature,
                "STEER",
                format_args!(
                    "mode={} side={:+.0} ship={:.0},{:.0} input={:.0},{:.0} new={:.0},{:.0}",
                    snapshot.mode(true),
                    snapshot.dodge_side,
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
    if command.is_null() || !crate::maneuvers_allowed(creature) {
        return;
    }
    let Some(now) = rd::<f64>(crate::CLOCK).filter(|t| t.is_finite()) else {
        return;
    };
    let Some(runtime) = RUNTIME.get().filter(|_| is_ai(creature)) else {
        apply_dodge_boost(creature, command, now);
        return;
    };
    let mode = {
        let mut ships = lock_ships();
        ships.by_creature.get_mut(&creature).and_then(|ship| {
            let (barrel_roll, side) = panic_math::engine_mode(ship, now)?;
            let flight = ship.flight;
            let trace_engine = runtime.config.log_panic && now - ship.trace_last_engine >= TRACE_PERIOD;
            if trace_engine {
                ship.trace_last_engine = now;
            }
            Some((barrel_roll, side, flight, trace_engine))
        })
    };
    let Some((barrel_roll, side, flight, trace_engine)) = mode else {
        apply_dodge_boost(creature, command, now);
        return;
    };

    let boost = runtime.config.panic_boost.max(1.0);
    if barrel_roll {
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
    if boost > 1.0 {
        if let Some(dir) = flight {
            if let Some(changed) = write_escape(command, dir, boost) {
                if trace_engine {
                    trace(
                        runtime,
                        now,
                        creature,
                        "ESCAPE",
                        format_args!(
                            "boost={boost} dir={:.3},{:.3} strength0={:.2}->{:.2} strength1={:.2}->{:.2}",
                            dir.0, dir.1, changed[0].0, changed[0].1, changed[1].0, changed[1].1
                        ),
                    );
                }
                return;
            }
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
}

/// The tactics' aim/dive boost. A dodge publishes the direction it steers along: both engine banks are then
/// pointed there and floored at the boost (a real run, not just a stronger command inside the stock velocity
/// controller). Without a direction the command strengths are raised only.
unsafe fn apply_dodge_boost(creature: usize, command: *mut f32, now: f64) {
    let (boost, escape) = crate::tactics::dodge_boost_for(creature, now);
    if boost <= 1.0 {
        return;
    }
    if let Some(dir) = escape {
        if write_escape(command, dir, boost).is_some() {
            return;
        }
    }
    for offset in [COMMAND_STRENGTH, COMMAND_NEXT_STRENGTH] {
        let strength = command.byte_add(offset);
        if let Some(boosted) = panic_math::boosted_strength(strength.read_unaligned(), boost) {
            strength.write_unaligned(boosted);
        }
    }
}

/// Sets the four engine-command angles in one go: the two force commands (`0x66` at `+0x10`,
/// `0x67` at `+0x30`) and the two swivel-servo commands (`0x64` at `-0x3c`, `0x65` at `-0x1c`).
unsafe fn write_engine_angles(command: *mut f32, first: f32, second: f32) {
    for (offset, angle) in [
        (COMMAND_ANGLE as isize, first),
        (COMMAND_NEXT_ANGLE as isize, second),
        (COMMAND_PREV2_ANGLE, first),
        (COMMAND_PREV_ANGLE, second),
    ] {
        command.byte_offset(offset).write_unaligned(angle);
    }
}

/// Command angle for a world push direction `(x, y)`: a part at world angle `a` pushes `(sin a, -cos a)`
/// (`Engine_ApplyThrustForce`), so `a = atan2(x, -y)`. Wrapped to `[0, 2pi)`, the range the AI tick uses.
fn direction_angle(direction: (f32, f32)) -> Option<f32> {
    let (x, y) = crate::tactics_math::normalized(direction.0, direction.1)?;
    let angle = x.atan2(-y);
    Some(if angle < 0.0 { angle + std::f32::consts::TAU } else { angle })
}

/// Dashes both engine banks along `direction` at `strength`: the player's Shift with the stick held.
/// This bypasses the stock AI's velocity controller and its `s * 25 * TWR` speed target, which no
/// command-strength raise can exceed. Returns the strengths before and after, `None` = direction unusable.
unsafe fn write_escape(command: *mut f32, direction: (f32, f32), strength: f32) -> Option<[(f32, f32); 2]> {
    let angle = direction_angle(direction)?;
    write_engine_angles(command, angle, angle);
    let mut changed = [(0.0f32, 0.0f32); 2];
    for (slot, offset) in [COMMAND_STRENGTH, COMMAND_NEXT_STRENGTH].into_iter().enumerate() {
        let field = command.byte_add(offset);
        let current = field.read_unaligned();
        field.write_unaligned(strength);
        changed[slot] = (current, strength);
    }
    Some(changed)
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

    write_engine_angles(command, first, second);
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
        if node == 0 || !plausible(node) || memory.read::<u8>(node + native_layout::NODE_CODE).map_or(true, |class| class & 0x2f != 0x2f) {
            return None;
        }
        memory.read::<f32>(node + CREATURE_SIDE).and_then(valid_side)
    };
    let Some(own) = side_of(memory, creature) else { return false };
    // aimed at something that is not a ship (a plane: the K-13; another missile): it is not coming for this ship, whoever
    // launched it (decided first: the log showed a player-side K-13 at a plane passing an enemy ship)
    if target != 0 && plausible(target) && memory.read::<u8>(target + native_layout::NODE_CODE).is_some_and(|class| class & 0x2f != 0x2f) {
        return true;
    }
    let owner = memory.read::<usize>(body + MISSILE_OWNER_LINK).unwrap_or(0);
    if let Some(launcher) = side_of(memory, owner) {
        return launcher.signum() == own.signum();
    }
    // the K-13 (carrier planes' air-to-air missile) stores its owner's side x 100 at +0x248, no usable owner link
    if let Some(marker) = memory.read::<f32>(body + 0x248).filter(|m| m.is_finite() && *m != 0.0 && m.abs() <= 10_000.0) {
        return marker.signum() == own.signum();
    }
    let aimed = side_of(memory, target).or_else(|| {
        let target_owner = memory.read::<usize>(target + NODE_OWNER).unwrap_or(0);
        side_of(memory, target_owner)
    });
    match aimed {
        Some(aimed) => aimed.signum() != own.signum(),
        // aimed at something that is not a ship (a plane, another missile): not coming for this ship
        None => target != 0 && plausible(target),
    }
}

/// The nearest live missile locked on to `creature` that is closer than the trigger range and whose reaction roll
/// has not already failed. Same scan as the original; every read is page-checked through a small region cache
/// ([`Region`]), the body count is capped, and the `is_active` function pointer must point into readable memory
/// before it is called.
unsafe fn threatened(runtime: &Runtime, creature: usize, ignored: &[usize]) -> ThreatScan {
    let mut memory = Region::new();
    let Some(count): Option<u32> = memory.read(runtime.body_count) else {
        return ThreatScan { threat: None, any_in_range: false };
    };
    let range = runtime.config.trigger_range;
    let mut nearest: Option<Threat> = None;
    let mut any_in_range = false;
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
        if category != CATEGORY_TACTICAL_MISSILE && category != CATEGORY_STRATEGIC_MISSILE {
            continue;
        }

        let target = memory.read::<usize>(body + BODY_TARGET).unwrap_or(0);
        let direct = target == creature;
        let locked = target != 0 && (direct || memory.read::<usize>(target + NODE_OWNER) == Some(creature));
        let mut stray = false;
        if !locked {
            // not locked on this ship: only a stray whose flight path crosses the ship counts (never its own missiles)
            if !stray_enabled || memory.read::<usize>(body + MISSILE_OWNER_LINK) == Some(creature) {
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
        if distance <= limit {
            any_in_range = true;
            if ignored.contains(&body) {
                continue;
            }
            if nearest.map_or(true, |best| distance < best.distance) {
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
                if !stray {
                    let logged = if direct {
                        &LOGGED_DIRECT_TARGET
                    } else {
                        &LOGGED_CHILD_TARGET
                    };
                    if !logged.swap(true, Ordering::Relaxed) {
                        log::info!(
                            "hf_ai ai_flares: first locked threat, missile link points at the {} (distance {distance:.0}, category {category:#x}, missile at {x:.0},{y:.0})",
                            if direct { "ship itself" } else { "a part of the ship" },
                        );
                    }
                }
            }
        }
    }

    ThreatScan { threat: nearest, any_in_range }
}
