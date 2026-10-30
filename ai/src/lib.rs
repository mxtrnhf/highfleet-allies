//! hf_ai: a tactical layer over the stock combat AI (Steam 1.163 only). Standalone DLL for the Highfleet Modloader.
//!
//! Plan (see `README.md` and `hf_notes/docs/findings/ai-telemetry.md`): the stock per-ship tick `FUN_140062f90` is kept;
//! this mod overrides its OUTPUTS (the steering waypoint at `+0xbf0/+0xbf4`, engine command strength, fire bytes,
//! launch gating) from a small set of tactics. Tactic logic is code; the numbers it uses live in `ai.json`.
//!
//! v0.4.0 adds "deny the dive" to the tactics. v0.3.0 added the `tactics` stage (`tactics.rs` glue, `tactics_math.rs` pure math): strafe in the band, off by default.
//!
//! v0.2.0 adds the port of qol's `ai_flares` (`ai_flares.rs`, config block `ai_flares`, off by default): after
//! [`decide`] has had its say about the waypoint, the missile-panic stage may override it (panic beats tactics), and
//! the flare-burst and engine-boost sites are hooked. See README.md.
//!
//! v0.1.0: the steering hook is in, **log-only**. Where the stock tick stores the waypoint it wants (`0x140064a8d`),
//! this mod sees the ship, the waypoint and the target, writes a CSV row per ship at `log_hz` and hands the waypoint
//! to [`decide`], which returns `None` (no change) until a tactic is written. Nothing in the game changes.
//!
//! The hook site is the same one qol's `ai_flares` uses for its evasion. Only one of the two can own it: the mod that
//! loads first wins (`hf_ai` sorts before `highfleet_qol`) and the other finds different bytes and skips its hook, with a
//! log line. With hf_ai installed this hook is ALWAYS installed (log-only unless `ai_flares` is on), so qol's
//! `ai_flares.evade` must be off even when hf_ai's `ai_flares` is off, and exactly one of the two `ai_flares` may be enabled.

use std::collections::HashMap;
use std::ffi::{c_char, CStr};
use std::fmt::Write as _;
use std::fs::File;
use std::io::Write as _;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use patchy::{PatchSession, ProcessModule, ReturnType, Trampoline};
use serde::{Deserialize, Serialize};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleA;
use windows_sys::Win32::System::Memory::{VirtualQuery, MEMORY_BASIC_INFORMATION};

mod ai_flares;
mod curtain_math;
mod flares_config;
mod missile_dump;
mod panic_math;
mod rng;
mod scan_log;
mod tactics;
mod tactics_config;
mod tactics_math;
mod twr_math;

use flares_config::AiFlaresConfig;
use tactics_config::TacticsConfig;

highfleet_mod_api::export_logger!();

const IMAGE_BASE: usize = 0x1_4000_0000;
const CONFIG_PATH: &str = "Modloader/config/hf_ai.json";

static GAME_IS_STEAM_1_163: AtomicBool = AtomicBool::new(false);

const STEER_RVA: usize = 0x64a8d;
const STEER_RESUME_RVA: usize = 0x64a96;
/// `movss [r14+0xbf0], xmm9` (the replaced instruction) and the `subss xmm9, xmm12` after it.
const STEER_BYTES: [u8; 14] = [0xf3, 0x45, 0x0f, 0x11, 0x8e, 0xf0, 0x0b, 0x00, 0x00, 0xf3, 0x45, 0x0f, 0x5c, 0xcc];
const STEER_STOLEN_LEN: usize = 9;
/// The AI tick's own first bytes (the hook site must be inside this function).
const TICK_RVA: usize = 0x62f90;
const TICK_PROLOGUE: [u8; 12] = [0x48, 0x8b, 0xc4, 0x55, 0x53, 0x41, 0x56, 0x48, 0x8d, 0xa8, 0xa8, 0xfe];

const CLOCK: usize = 0x1_43a1_19c0; // f64 game clock
const OFF_SIDE: usize = 0xad4; // f32: negative = AI, positive = player's side
const OFF_TARGET: usize = 0xb00; // pointer: target node (a ship or a part of one)
const OFF_POS_X: usize = 0x1840; // f32 centre of mass
const OFF_POS_Y: usize = 0x1844;
const OFF_OWNER: usize = 0xb8;
const OFF_CODE: usize = 0x28;

/// Scratch space shared with the steering trampoline: waypoint x, y and ship x, y.
static STEER_BUFFER: [AtomicU32; 4] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];
static LOG_HZ: AtomicU32 = AtomicU32::new(0x4120_0000); // 10.0f32
static LOG_STEERING: AtomicBool = AtomicBool::new(true);

struct Logger {
    file: Option<File>,
    /// battle id (hf_ally's session id from `Modloader/current_battle.txt`) the open file belongs to
    battle: u64,
    /// false while hf_ally's session for `battle` is closed or packed: no rows, no file
    active: bool,
    /// game time of the next look at `current_battle.txt`
    next_battle_check: f64,
    last_clock: f64,
    /// ship pointer -> (small id, game time of its last row)
    ships: HashMap<usize, (u32, f64)>,
}

static LOGGER: Mutex<Option<Logger>> = Mutex::new(None);

/// What the tactic layer gets to look at when the stock AI has picked a waypoint.
#[allow(dead_code)]
struct Ctx {
    now: f64,
    creature: usize,
    side: f32,
    ship: (f32, f32),
    /// The waypoint the stock AI wants (200 units from the ship along the wanted direction).
    wanted: (f32, f32),
    /// Position of the ship the AI is shooting at, if it has a target.
    target: Option<(f32, f32)>,
    /// The creature behind that target node.
    target_creature: Option<usize>,
}

/// The battle hf_ally is recording (`Modloader/current_battle.txt`) and whether it is being recorded now. hf_ally writes
/// the session id when it arms a battle and `0` when the session closes (before the packer runs), so `0` means "no
/// session": hf_ai logs nothing then, and nothing for an id whose `hf_ally_telemetry_<id>.csv` has left `logs/`
/// (the packer moved it). Without the file at all (no hf_ally installed) hf_ai logs, naming files by its own stamp (id 0).
pub(crate) fn battle_state() -> (u64, bool) {
    let Some(id) = std::fs::read_to_string("Modloader/current_battle.txt").ok().and_then(|text| text.trim().parse::<u64>().ok()) else {
        return (0, true);
    };
    let active = id != 0 && std::path::Path::new(&format!("Modloader/logs/hf_ally_telemetry_{id}.csv")).exists();
    (id, active)
}

/// Page-checked read (same test as `hf_ally`'s `diag::rd`): `None` when the memory is not mapped and readable.
unsafe fn rd<T: Copy>(address: usize) -> Option<T> {
    let end = address.checked_add(std::mem::size_of::<T>())?;
    if address < 0x1_0000 || !readable(address) || (end - 1) / 4096 != address / 4096 && !readable(end - 1) {
        return None;
    }
    Some((address as *const T).read_unaligned())
}

unsafe fn readable(address: usize) -> bool {
    let mut info: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
    if VirtualQuery(address as *const std::ffi::c_void, &mut info, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) == 0 {
        return false;
    }
    // MEM_COMMIT, and not PAGE_NOACCESS (0x01) / PAGE_GUARD (0x100)
    info.State == 0x1000 && info.Protect & 0x101 == 0 && info.Protect != 0
}

fn plausible(p: usize) -> bool {
    p >= 0x10000 && p < 0x7fff_ffff_0000 && p % 8 == 0
}

/// The creature behind a target node (the node is a ship or one of its parts) and its position.
unsafe fn target_info(node: usize) -> Option<(usize, (f32, f32))> {
    if !plausible(node) {
        return None;
    }
    let ship = if rd::<u8>(node + OFF_CODE)? & 0x2f == 0x2f {
        node
    } else {
        let owner: usize = rd(node + OFF_OWNER)?;
        if !plausible(owner) || rd::<u8>(owner + OFF_CODE)? & 0x2f != 0x2f {
            return None;
        }
        owner
    };
    Some((ship, (rd(ship + OFF_POS_X)?, rd(ship + OFF_POS_Y)?)))
}

/// Called from the AI tick right before it stores the waypoint it wants for `creature`.
/// Stages: the stock waypoint -> [`decide`] (tactics) -> the ported missile panic (`ai_flares::steer`, which overrides
/// tactics while a ship is evading) -> the CSV row -> the waypoint goes back to the tick.
unsafe extern "C" fn on_steering(creature: usize) {
    // A Rust panic must never unwind into the game.
    if catch_unwind(AssertUnwindSafe(|| steering(creature))).is_err() && !LOGGED_STEER_PANIC.swap(true, Ordering::Relaxed) {
        log::error!("hf_ai: a panic was caught in the steering hook and ignored");
    }
}

static LOGGED_STEER_PANIC: AtomicBool = AtomicBool::new(false);

static TWR_CUTOFF: AtomicU32 = AtomicU32::new(0);

/// Steam 1.163 battle TWR, not map TWR. See hf_notes/docs/findings/ship-profiles.md;
/// confirmed against Creature_HasTWRStat (0x140073450) in the live Ghidra program.
fn maneuvers_allowed(creature: usize) -> bool {
    let cutoff = f32::from_bits(TWR_CUTOFF.load(Ordering::Relaxed));
    if cutoff == 0.0 {
        return true;
    }
    // SAFETY: page-checked read; missing/invalid TWR fails the enabled gate.
    twr_math::allows(unsafe { rd::<f32>(creature + 0x1604) }, cutoff)
}


unsafe fn steering(creature: usize) {
    let load = |index: usize| f32::from_bits(STEER_BUFFER[index].load(Ordering::Relaxed));
    let (wanted, ship) = ((load(0), load(1)), (load(2), load(3)));
    let Some(now) = rd::<f64>(CLOCK) else { return };
    let info = rd(creature + OFF_TARGET).and_then(|node| target_info(node));
    let ctx = Ctx {
        now,
        creature,
        side: rd(creature + OFF_SIDE).unwrap_or(0.0),
        ship,
        wanted,
        target: info.map(|(_, position)| position),
        target_creature: info.map(|(creature, _)| creature),
    };
    let (tactic, tactic_state) = tactics::decide(&ctx, || ai_flares::panicking(creature, now));
    // The panic stage sees the waypoint the tick will store (the tactic's, if it chose one) and overrides it.
    let tactic_waypoint = match tactic {
        Some((x, y)) if x.is_finite() && y.is_finite() => (x, y),
        _ => wanted,
    };
    // (returns None at once when ai_flares is off: its runtime is never created then)
    let panic = ai_flares::steer(creature, now, tactic_waypoint, ship).filter(|(x, y)| x.is_finite() && y.is_finite());
    let chosen = panic.or(tactic);
    if LOG_STEERING.load(Ordering::Relaxed) {
        let state = if panic.is_some() { tactics_math::State::Panic } else { tactic_state };
        log_row(&ctx, chosen, panic.is_some(), state);
    }
    if let Some((x, y)) = chosen {
        if x.is_finite() && y.is_finite() {
            STEER_BUFFER[0].store(x.to_bits(), Ordering::Relaxed);
            STEER_BUFFER[1].store(y.to_bits(), Ordering::Relaxed);
        }
    }
}

fn log_row(ctx: &Ctx, chosen: Option<(f32, f32)>, panic: bool, state: tactics_math::State) {
    let Ok(mut guard) = LOGGER.try_lock() else { return };
    let log = guard.get_or_insert_with(|| Logger { file: None, battle: 0, active: true, next_battle_check: 0.0, last_clock: 0.0, ships: HashMap::new() });
    if ctx.now < log.last_clock {
        log.ships.clear(); // the clock went back (it normally runs on across battles)
    }
    log.last_clock = ctx.now;
    // One file per battle, named with hf_ally's session id so the .hftm packer finds it. The game clock does not
    // restart between battles, so the battle is told by `current_battle.txt` (looked at twice a second of game time).
    if ctx.now >= log.next_battle_check || ctx.now < log.next_battle_check - 5.0 {
        log.next_battle_check = ctx.now + 0.5;
        let (battle, active) = battle_state();
        if battle != log.battle || active != log.active {
            log.battle = battle;
            log.active = active;
            log.file = None; // closed; the next row opens the new battle's file
            log.ships.clear();
        }
    }
    if !log.active {
        return;
    }
    let period = 1.0 / f64::from(f32::from_bits(LOG_HZ.load(Ordering::Relaxed)).max(0.1));
    let next_id = log.ships.len() as u32 + 1;
    let entry = log.ships.entry(ctx.creature).or_insert((next_id, f64::NEG_INFINITY));
    if ctx.now - entry.1 < period {
        return;
    }
    entry.1 = ctx.now;
    let id = entry.0;
    if log.file.is_none() {
        let stamp = if log.battle != 0 {
            log.battle
        } else {
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
        };
        let opened = std::fs::OpenOptions::new().create(true).append(true).open(format!("Modloader/logs/hf_ai_steer_{stamp}.csv"));
        if let Ok(mut file) = opened {
            let _ = writeln!(
                file,
                "# hf_ai v{}: the stock AI's wanted waypoint per ship. wang = direction of the wanted waypoint from the ship (deg),                  tbear = bearing to the target (deg), rel = wang - tbear wrapped to -180..180 (0 = heading straight at the target,                  +-90 = circling). chosen_x/y empty = hf_ai left the waypoint alone, panic = 1 when the ported ai_flares evasion chose it, cptr = creature pointer (joins with hf_ai_panic_*.log), state = stock (a gate failed) / far / band / close (tactics, chosen_x/y set when it overrode) / panic. dist is the range to the target.",
                env!("CARGO_PKG_VERSION")
            );
            let _ = writeln!(file, "t,ship,side,x,y,wx,wy,wlen,wang,tx,ty,dist,tbear,rel,chosen_x,chosen_y,panic,cptr,state");
            log.file = Some(file);
        }
    }
    let wang = (ctx.wanted.1 - ctx.ship.1).atan2(ctx.wanted.0 - ctx.ship.0).to_degrees();
    let wlen = ((ctx.wanted.0 - ctx.ship.0).powi(2) + (ctx.wanted.1 - ctx.ship.1).powi(2)).sqrt();
    let mut row = String::new();
    let _ = write!(row, "{:.3},{id},{:.0},{:.1},{:.1},{:.1},{:.1},{wlen:.0},{wang:.1},", ctx.now, ctx.side, ctx.ship.0, ctx.ship.1, ctx.wanted.0, ctx.wanted.1);
    match ctx.target {
        Some((tx, ty)) => {
            let dist = ((tx - ctx.ship.0).powi(2) + (ty - ctx.ship.1).powi(2)).sqrt();
            let tbear = (ty - ctx.ship.1).atan2(tx - ctx.ship.0).to_degrees();
            let rel = (wang - tbear + 540.0).rem_euclid(360.0) - 180.0;
            let _ = write!(row, "{tx:.1},{ty:.1},{dist:.0},{tbear:.1},{rel:.1},");
        }
        None => row.push_str(",,,,,"),
    }
    match chosen {
        Some((x, y)) => {
            let _ = write!(row, "{x:.1},{y:.1}");
        }
        None => row.push(','),
    }
    let _ = write!(row, ",{},0x{:x},{}", u8::from(panic), ctx.creature, state.name());
    row.push('\n');
    if let Some(file) = log.file.as_mut() {
        let _ = file.write_all(row.as_bytes());
    }
}

/// `mov rax, imm64` of the scratch buffer's address.
fn load_buffer_address(code: &mut Vec<u8>) {
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&(STEER_BUFFER.as_ptr() as u64).to_le_bytes());
}

/// Copies the waypoint (`xmm9`, `xmm10`) and ship position (`xmm12`, `xmm14`) into the scratch buffer and passes the
/// creature (`r14`) to the hook.
fn steer_argument_setup() -> Vec<u8> {
    let mut code = Vec::new();
    load_buffer_address(&mut code);
    code.extend_from_slice(&[
        0xf3, 0x44, 0x0f, 0x11, 0x08, // movss [rax], xmm9
        0xf3, 0x44, 0x0f, 0x11, 0x50, 0x04, // movss [rax+4], xmm10
        0xf3, 0x44, 0x0f, 0x11, 0x60, 0x08, // movss [rax+8], xmm12
        0xf3, 0x44, 0x0f, 0x11, 0x70, 0x0c, // movss [rax+12], xmm14
        0x4c, 0x89, 0xf1, // mov rcx, r14
    ]);
    code
}

/// Loads the (possibly changed) waypoint back into `xmm9` and `xmm10`.
fn steer_result_handling() -> Vec<u8> {
    let mut code = Vec::new();
    load_buffer_address(&mut code);
    code.extend_from_slice(&[
        0xf3, 0x44, 0x0f, 0x10, 0x08, // movss xmm9, [rax]
        0xf3, 0x44, 0x0f, 0x10, 0x50, 0x04, // movss xmm10, [rax+4]
    ]);
    code
}

/// Whether the AI tick and the steering site are what was analysed. The error line names the other-mod case.
unsafe fn steering_site_ok(module: ProcessModule) -> Result<bool, String> {
    let matches = |rva: usize, expected: &[u8]| -> Result<bool, String> {
        let address = module.resolve_rva(rva).map_err(|e| e.to_string())?;
        Ok(std::slice::from_raw_parts(address as *const u8, expected.len()) == expected)
    };
    if !matches(TICK_RVA, &TICK_PROLOGUE)? {
        log::error!("hf_ai: the AI tick at {TICK_RVA:#x} is not what was analysed, no steering hook");
        return Ok(false);
    }
    if !matches(STEER_RVA, &STEER_BYTES)? {
        log::error!(
            "hf_ai: the steering site {STEER_RVA:#x} has other bytes ({}), no steering hook",
            ai_flares::OWNERSHIP_HINT
        );
        return Ok(false);
    }
    Ok(true)
}

/// Prepares the steering detour in `session` (installed by the caller).
unsafe fn prepare_steering_hook(session: &mut PatchSession, module: ProcessModule) -> Result<(), String> {
    let mut trampoline = Trampoline::new();
    trampoline.preserved_call(on_steering as *const (), &steer_argument_setup(), &steer_result_handling(), ReturnType::None);
    // replay the replaced store of the (possibly changed) waypoint
    trampoline.bytes(&STEER_BYTES[..STEER_STOLEN_LEN]);
    trampoline.absolute_jump(module.resolve_rva(STEER_RESUME_RVA).map_err(|e| e.to_string())?);
    session
        .detour_trampoline(module.resolve_rva(STEER_RVA).map_err(|e| e.to_string())?, STEER_STOLEN_LEN, trampoline)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Config {
    /// Master switch.
    enabled: bool,
    /// Minimum battle TWR for evasion, stray-missile reactions and tactics; 0 disables the gate.
    twr_cutoff: f32,
    /// Write `Modloader/logs/hf_ai_steer_<unix>.csv`: the stock AI's wanted waypoint per ship.
    log_steering: bool,
    /// Rows per second per ship in that file.
    log_hz: f32,
    /// Write `hf_ai_scan_<id>.csv` / `hf_ai_obst_<id>.csv`: every AI scan call with the obstacle nodes and the steering
    /// numbers the game computed (50 Hz per ship). Off by default; see `scan_log.rs`.
    log_scan: bool,
    /// With `log_scan`: also list every body the scan considered (only on ticks with a kept node).
    log_scan_bodies: bool,
    /// The ported qol `ai_flares` (flares burst, sidestep, panic evasion, barrel roll, allies). Off by default:
    /// enable it only with qol's `ai_flares.enabled` (and `evade`) turned off.
    ai_flares: AiFlaresConfig,
    /// "Strafe in the band" enemy positioning (v0.3.0). Off by default.
    tactics: TacticsConfig,
    /// Chance (0..1) that an enemy ship keeps launching Zenith missiles right after a launch (a burst that empties the
    /// rack) instead of waiting for the game's 2 s cooldown. 0 = vanilla. Was `ai_missile_dump_chance` in qol.json.
    ai_missile_dump_chance: f32,
    /// Shortest and longest wait between the launches of a burst, in seconds (`max` never below `min`).
    ai_missile_dump_min_delay: f32,
    ai_missile_dump_max_delay: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self { enabled: true, twr_cutoff: 0.0, log_steering: true, log_hz: 10.0, log_scan: false, log_scan_bodies: false, ai_flares: AiFlaresConfig::default(), tactics: TacticsConfig::default(), ai_missile_dump_chance: 0.0, ai_missile_dump_min_delay: missile_dump::DEFAULT_MIN_DELAY, ai_missile_dump_max_delay: missile_dump::DEFAULT_MAX_DELAY }
    }
}


/// Warns about the QoL mod setup that hf_ai relies on: logdot's `highfleet-qol` carries the 1.163 flare crash fix (a
/// proximity-fused missile meeting a flare whose link is gone), and hf_ai drops far more flares than the stock game.
/// Checked from the files (mod load order is alphabetical, so the QoL DLL may not be loaded yet when hf_ai starts).
fn check_qol(config: &Config) {
    let has_dll = std::fs::read_dir("Modloader/mods")
        .map(|dir| {
            dir.flatten().any(|entry| {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                name.contains("qol") && name.ends_with(".dll")
            })
        })
        .unwrap_or(false);
    if !has_dll {
        log::warn!("hf_ai: no QoL mod in Modloader/mods. Install logdot's highfleet-qol: it has the flare crash fix, and hf_ai's flares make that crash much more likely without it");
        return;
    }
    let Ok(text) = std::fs::read_to_string("Modloader/config/qol.json") else { return };
    let Ok(qol) = serde_json::from_str::<serde_json::Value>(&text) else { return };
    if qol.get("enable_flare_crash_fix").and_then(|v| v.as_bool()) == Some(false) {
        log::warn!("hf_ai: qol.json has enable_flare_crash_fix false: turn it on, hf_ai's flares can crash the game without it");
    }
    if config.ai_missile_dump_chance > 0.0 && qol.get("ai_missile_dump_chance").and_then(|v| v.as_f64()).unwrap_or(0.0) > 0.0 {
        log::warn!("hf_ai: qol.json ai_missile_dump_chance is above 0 as well: set it to 0, hf_ai does the missile dump");
    }
    if qol.get("ai_flares").and_then(|v| v.get("enabled")).and_then(|v| v.as_bool()) == Some(true) && config.ai_flares.enabled {
        log::warn!("hf_ai: qol.json ai_flares.enabled is true as well: set it to false, both patch the same sites");
    }
}

#[no_mangle]
unsafe extern "C" fn version(version: *const c_char) -> bool {
    let supported = CStr::from_ptr(version).to_bytes() == b"Steam 1.163";
    GAME_IS_STEAM_1_163.store(supported, Ordering::Relaxed);
    supported
}

#[no_mangle]
unsafe extern "C" fn init() -> bool {
    log::info!("hf_ai v{} (build {})", env!("CARGO_PKG_VERSION"), env!("HF_AI_BUILD"));
    if !GAME_IS_STEAM_1_163.load(Ordering::Relaxed) {
        log::error!("hf_ai: needs Steam 1.163, not starting");
        return false;
    }
    if GetModuleHandleA(std::ptr::null()) as usize != IMAGE_BASE {
        log::error!("hf_ai: unexpected image base, not starting");
        return false;
    }
    let config: Config = match std::fs::read_to_string(CONFIG_PATH) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
            log::error!("hf_ai: bad {CONFIG_PATH}: {error}; using defaults");
            Config::default()
        }),
        Err(_) => {
            let config = Config::default();
            if let Ok(text) = serde_json::to_string_pretty(&config) {
                let _ = std::fs::write(CONFIG_PATH, text);
            }
            config
        }
    };
    if !config.enabled {
        log::info!("hf_ai: disabled in {CONFIG_PATH}");
        return true;
    }
    check_qol(&config);
    let cutoff = twr_math::sanitize_cutoff(config.twr_cutoff);
    TWR_CUTOFF.store(cutoff.to_bits(), Ordering::Relaxed);
    log::info!("hf_ai: maneuver TWR cutoff {cutoff}");
    tactics::init(&config.tactics);
    LOG_STEERING.store(config.log_steering, Ordering::Relaxed);
    scan_log::LOG_BODIES.store(config.log_scan_bodies, Ordering::Relaxed);
    LOG_HZ.store(config.log_hz.clamp(0.5, 60.0).to_bits(), Ordering::Relaxed);
    let module = match ProcessModule::main() {
        Ok(module) => module,
        Err(error) => {
            log::error!("hf_ai: {error}");
            return false;
        }
    };
    let steering_ok = match steering_site_ok(module) {
        Ok(ok) => ok,
        Err(error) => {
            log::error!("hf_ai: cannot check the steering site: {error}");
            false
        }
    };
    // One session for every site, so nothing is half installed. Preparing the ai_flares sites may fail (a code
    // mismatch is not an error, it just leaves ai_flares off); then the steering hook is prepared alone.
    let prepare = |with_flares: bool, with_flares_or_scan: bool| -> Result<PatchSession, String> {
        let mut session = PatchSession::new();
        if steering_ok {
            prepare_steering_hook(&mut session, module)?;
        }
        if config.log_scan && steering_ok && with_flares_or_scan {
            match scan_log::prepare(&mut session, module) {
                Ok(true) => log::info!("hf_ai: scan log hooks prepared"),
                Ok(false) => {}
                Err(error) => return Err(error),
            }
        }
        {
            missile_dump::patch_ai_missile_dump(
                &mut session,
                config.ai_missile_dump_chance,
                config.ai_missile_dump_min_delay,
                config.ai_missile_dump_max_delay,
            )
            .map_err(|e| e.to_string())?;
        }
        if with_flares {
            ai_flares::patch_ai_flares(&mut session, &config.ai_flares, steering_ok).map_err(|e| e.to_string())?;
        }
        Ok(session)
    };
    let session = match prepare(config.ai_flares.enabled, true) {
        Ok(session) => session,
        Err(error) => {
            log::error!("hf_ai: preparing the hooks failed: {error}");
            if !config.ai_flares.enabled {
                return false;
            }
            log::error!("hf_ai: retrying with the steering hook only (ai_flares off)");
            match prepare(false, false) {
                Ok(session) => session,
                Err(error) => {
                    log::error!("hf_ai: preparing the steering hook failed: {error}");
                    return false;
                }
            }
        }
    };
    if let Err(error) = session.install_permanently() {
        log::error!("hf_ai: installing the hooks failed: {error}");
        return false;
    }
    if steering_ok {
        log::info!(
            "hf_ai: steering hook installed (tactics {}; ai_flares {})",
            if config.tactics.enabled { "on" } else { "off" },
            if config.ai_flares.enabled { "requested, see the ai_flares lines above" } else { "off" }
        );
    }
    if !config.ai_flares.enabled {
        log::info!("hf_ai: ai_flares disabled in {CONFIG_PATH} (keep qol ai_flares evade OFF while hf_ai's steering hook is installed)");
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twr_cutoff_is_optional_and_configurable() {
        let old: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(old.twr_cutoff, 0.0);
        let configured: Config = serde_json::from_str(r#"{"twr_cutoff":3.5}"#).unwrap();
        assert_eq!(configured.twr_cutoff, 3.5);
        assert_eq!(serde_json::to_value(&configured).unwrap()["twr_cutoff"], 3.5);
    }


    /// Writes the full default config to the path in `HF_DUMP_CONFIG` (used to produce the example config of the release pack).
    #[test]
    #[ignore]
    fn dump_default_config() {
        if let Ok(path) = std::env::var("HF_DUMP_CONFIG") {
            std::fs::write(path, serde_json::to_string_pretty(&Config::default()).unwrap()).unwrap();
        }
    }

    #[test]
    fn old_config_without_ai_flares_loads_with_it_off() {
        let config: Config = serde_json::from_str(r#"{"enabled": true, "log_steering": false}"#).unwrap();
        assert!(!config.log_steering && !config.ai_flares.enabled);
        assert_eq!(config.ai_flares.trigger_range, 400.0);
        let text = serde_json::to_string_pretty(&Config::default()).unwrap();
        assert!(text.contains("\"ai_flares\"") && text.contains("\"log_panic\": false"));
        let nested: Config = serde_json::from_str(r#"{"ai_flares": {"enabled": true, "log_panic": true}}"#).unwrap();
        assert!(nested.ai_flares.enabled && nested.ai_flares.log_panic && nested.ai_flares.evade && nested.ai_flares.allies);
    }

    #[test]
    fn v0_2_0_config_without_tactics_loads_with_tactics_off() {
        let config: Config = serde_json::from_str(r#"{"enabled": true, "ai_flares": {"enabled": true}}"#).unwrap();
        assert!(!config.tactics.enabled && config.ai_flares.enabled);
        assert!(!config.tactics.params().enabled);
        let text = serde_json::to_string_pretty(&Config::default()).unwrap();
        assert!(text.contains("\"tactics\"") && text.contains("\"k_center\""));
        let on: Config = serde_json::from_str(r#"{"tactics": {"enabled": true}}"#).unwrap();
        assert!(on.tactics.params().enabled && !on.ai_flares.enabled);
    }

    #[test]
    fn steering_trampoline_round_trips_the_waypoint_and_replays_the_store() {
        let module_base = 0x140000000;
        let mut trampoline = Trampoline::new();
        trampoline.preserved_call(on_steering as *const (), &steer_argument_setup(), &steer_result_handling(), ReturnType::None);
        trampoline.bytes(&STEER_BYTES[..STEER_STOLEN_LEN]);
        trampoline.absolute_jump(ProcessModule::from_base(module_base).resolve_rva(STEER_RESUME_RVA).unwrap());
        let trampoline = trampoline.build(0x180000000).unwrap();

        let setup = steer_argument_setup();
        let result = steer_result_handling();
        let setup_at = trampoline.windows(setup.len()).position(|window| window == setup).expect("waypoint copied out");
        let result_at = trampoline.windows(result.len()).position(|window| window == result).expect("waypoint loaded back");
        let store_at = trampoline
            .windows(STEER_STOLEN_LEN)
            .position(|window| window == &STEER_BYTES[..STEER_STOLEN_LEN])
            .expect("the replaced store is replayed");
        assert!(setup_at < result_at && result_at < store_at);
        assert_eq!(
            usize::from_le_bytes(trampoline[trampoline.len() - 8..].try_into().unwrap()),
            module_base + STEER_RESUME_RVA
        );
        assert_eq!(STEER_RVA + STEER_STOLEN_LEN, STEER_RESUME_RVA);
    }
}
