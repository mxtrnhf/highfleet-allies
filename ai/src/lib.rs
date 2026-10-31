//! hf_ai: a tactical layer over the stock combat AI (Steam 1.163 only). Standalone DLL for the Highfleet Modloader.
//!
//! Plan (see `README.md` and `hf_notes/docs/findings/ai-telemetry.md`): the stock per-ship tick `FUN_140062f90` is kept;
//! this mod overrides its OUTPUTS (the steering waypoint at `+0xbf0/+0xbf4`, engine command strength, fire bytes,
//! launch gating) from a small set of tactics. Tactic logic is code; the numbers it uses live in `ai.json`.
//!
//! The tactics stage retains aim dodging and dive avoidance. Optional strafe/band positioning was retired in v0.10.6.
//!
//! v0.2.0 adds the port of qol's `ai_flares` (`ai_flares.rs`, config block `ai_flares`, off by default): after
//! [`decide`] has had its say about the waypoint, the missile-panic stage may override it (panic beats tactics), and
//! the flare-burst and engine-boost sites are hooked. See README.md.
//!
//! The hook site is the same one qol's `ai_flares` uses for its evasion. Only one of the two can own it: the mod that
//! loads first wins (`hf_ai` sorts before `highfleet_qol`) and the other finds different bytes and skips its hook, with a
//! log line. With hf_ai installed this hook is ALWAYS installed, so qol's
//! `ai_flares.evade` must be off even when hf_ai's `ai_flares` is off, and exactly one of the two `ai_flares` may be enabled.

use std::ffi::{c_char, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use patchy::{PatchSession, ProcessModule, ReturnType, Trampoline};
use serde::{Deserialize, Serialize};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleA;
use windows_sys::Win32::System::Memory::{VirtualQuery, MEMORY_BASIC_INFORMATION};

mod ai_flares;
mod curtain_math;
mod flares_config;
mod missile_dump;
mod panic_math;
mod plane_group;
mod plane_group_math;
mod rng;
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
/// tactics while a ship is evading) -> the waypoint goes back to the tick.
unsafe extern "C" fn on_steering(creature: usize) {
    // A Rust panic must never unwind into the game.
    if catch_unwind(AssertUnwindSafe(|| steering(creature))).is_err() && !LOGGED_STEER_PANIC.swap(true, Ordering::Relaxed) {
        log::error!("hf_ai: a panic was caught in the steering hook and ignored");
    }
}

static LOGGED_STEER_PANIC: AtomicBool = AtomicBool::new(false);

static TWR_CUTOFF: AtomicU32 = AtomicU32::new(0);

/// Steam 1.163 strategic TWR shown in Shipworks, not battle TWR (+0x1604).
/// Verified displayinShipworks? reads +0x160c at 0x1400d7443 for TWR_STAT.
/// See hf_notes/docs/findings/ship-profiles.md; highfleet-rs has no Creature layout.
fn strategic_twr(creature: usize) -> Option<f32> {
    // SAFETY: borrowed scalar only; rd page-checks the game-owned field.
    unsafe { rd::<f32>(creature + 0x160c) }
}

fn maneuvers_allowed(creature: usize) -> bool {
    let cutoff = f32::from_bits(TWR_CUTOFF.load(Ordering::Relaxed));
    if cutoff == 0.0 {
        return true;
    }
    // Missing/invalid strategic TWR fails the enabled gate.
    twr_math::allows(strategic_twr(creature), cutoff)
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
    let (tactic, _) = tactics::decide(&ctx, || ai_flares::panicking(creature, now));
    // The panic stage sees the waypoint the tick will store (the tactic's, if it chose one) and overrides it.
    let tactic_waypoint = match tactic {
        Some((x, y)) if x.is_finite() && y.is_finite() => (x, y),
        _ => wanted,
    };
    // (returns None at once when ai_flares is off: its runtime is never created then)
    let panic = ai_flares::steer(creature, now, tactic_waypoint, ship).filter(|(x, y)| x.is_finite() && y.is_finite());
    let chosen = panic.or(tactic);
    if let Some((x, y)) = chosen {
        if x.is_finite() && y.is_finite() {
            STEER_BUFFER[0].store(x.to_bits(), Ordering::Relaxed);
            STEER_BUFFER[1].store(y.to_bits(), Ordering::Relaxed);
        }
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

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct LoggingConfig {
    /// Text event trace for flare/panic decisions (not battle capture).
    log_panic: bool,
}

/// Accept the old ai_flares.log_panic key; the logging section takes precedence.
fn parse_config(text: &str) -> Result<Config, serde_json::Error> {
    let mut value: serde_json::Value = serde_json::from_str(text)?;
    if let Some(root) = value.as_object_mut() {
        let legacy = root
            .get_mut("ai_flares")
            .and_then(|flares| flares.as_object_mut())
            .and_then(|flares| flares.remove("log_panic"));
        if let Some(setting) = legacy {
            let logging = root
                .entry("logging")
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
            if let Some(logging) = logging.as_object_mut() {
                logging.entry("log_panic").or_insert(setting);
            }
        }
    }
    let mut config: Config = serde_json::from_value(value)?;
    config.ai_flares.log_panic = config.logging.log_panic;
    Ok(config)
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Config {
    /// Master switch.
    enabled: bool,
    /// Planes per tactical AI carrier launch group (1 = stock; limited to 1..64).
    plane_group_per_launch: i32,
    /// Seconds between group members (limited to 0.05..60; default 0.5).
    plane_group_launch_delay_seconds: f32,
    /// Minimum strategic/Shipworks TWR for maneuvers; 0 disables the gate.
    twr_cutoff: f32,
    logging: LoggingConfig,
    /// The ported qol `ai_flares` (flares burst, sidestep, panic evasion, barrel roll, allies). Enabled by player default:
    /// use it only with qol's `ai_flares.enabled` (and `evade`) turned off.
    ai_flares: AiFlaresConfig,
    /// Aim dodging and dive avoidance. Off by default.
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
        Self {
            enabled: true,
            plane_group_per_launch: plane_group_math::DEFAULT_GROUP_SIZE,
            plane_group_launch_delay_seconds: plane_group_math::DEFAULT_DELAY,
            twr_cutoff: 2.0,
            logging: LoggingConfig::default(),
            ai_flares: AiFlaresConfig::default(),
            tactics: TacticsConfig::default(),
            ai_missile_dump_chance: 0.4,
            ai_missile_dump_min_delay: 0.6,
            ai_missile_dump_max_delay: 1.5,
        }
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
        Ok(text) => parse_config(&text).unwrap_or_else(|error| {
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
    log::info!("hf_ai: maneuver strategic/Shipworks TWR cutoff {cutoff}");
    tactics::init(&config.tactics);
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
    let prepare = |with_flares: bool| -> Result<PatchSession, String> {
        let mut session = PatchSession::new();
        if steering_ok {
            prepare_steering_hook(&mut session, module)?;
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
        plane_group::prepare(
            &mut session, module, config.plane_group_per_launch,
            config.plane_group_launch_delay_seconds,
        )?;
        if with_flares {
            ai_flares::patch_ai_flares(&mut session, &config.ai_flares, steering_ok).map_err(|e| e.to_string())?;
        }
        Ok(session)
    };
    let session = match prepare(config.ai_flares.enabled) {
        Ok(session) => session,
        Err(error) => {
            log::error!("hf_ai: preparing the hooks failed: {error}");
            if !config.ai_flares.enabled {
                return false;
            }
            log::error!("hf_ai: retrying with the steering hook only (ai_flares off)");
            match prepare(false) {
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
