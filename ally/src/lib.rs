//! PROTOTYPE, standalone. A player-side ship that fights beside you, run by the stock AI
//! (Steam 1.163 only). Background and addresses: `hf_notes/docs/findings/battle-ally.md`.
//!
//! v0.2.0 added the **ally** (always enabled with the mod): once per battle, when the player's
//! ship has been flying for `ally_delay_seconds` and the fleet still has a ship waiting in the
//! reserve, the vanilla deploy function is called for a spare control slot and the new ship is
//! handed back to the stock AI as a wingman (see the findings note, "Revised plan").
//!
//! v0.1.x is **log-only**: two entry hooks, nothing in the game changes.
//!
//! * `ALLY ADD|REMOVE`: the Creature add-to-level routine, one line per creature, with the fields
//!   that decide whether the stock AI runs it and which side it is on.
//! * `ALLY SPAWN`: the ship-from-template spawner, with the **return address** of its caller, so
//!   one Combat run names the call site that deploys the player's ships (the findings note lists
//!   22 candidates) and the template it spawns from.
//!
//! Every line carries the pending level name (`Combat.seria`, `Missile.seria`, ...); the line
//! budget is per level, so shipworks and map traffic cannot use it up before a battle.
//!
//! Config: `Modloader/config/ally.json` (created with defaults on first run).

highfleet_mod_api::export_logger!();

/// A line for the debug mode's megalog (a no-op, and the arguments are not even formatted, otherwise).
macro_rules! crumb {
    ($($arg:tt)*) => {
        if diag::debug() {
            diag::crumb(&format!($($arg)*));
        }
    };
}

mod aim_math;
mod ally_ammo;
mod ally_hud;
mod ally_panels;
mod ally_reticles;
mod battle;
mod battle_diagnostics;
mod bomber;
mod bomber_math;
mod config;
mod debug_overlay;
mod debug_state;
mod diag;
mod enemy_waves;
mod honor_share;
mod intercept;
mod missile_guard;
mod native;
mod native_layout;
mod plane_loadouts;
mod precombat;
mod ship_switch;
mod sound;

use std::{
    ffi::{c_char, CStr},
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Mutex,
    },
};

use config::{parse_hex_color, Config};
use native::{
    count, read, CLOCK, CONTROLLED_LINK, CREATURES_BEGIN, CREATURES_END, GET_PROFILE,
    GET_PROFILE_PROLOGUE, IMAGE_BASE, OFF_AGGRESSION, OFF_AI_FLAG, OFF_MODE, OFF_SIDE, OFF_STATE,
    OFF_TARGET, SCENE_FLAGS, SCENE_SELECTOR, SET_CONTROLLED, SET_CONTROLLED_PROLOGUE, SET_LINK,
    SET_LINK_PROLOGUE,
};
use patchy::{PatchSession, ProcessModule, ReturnType, Trampoline};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleA;

/// `FUN_14006bf60(creature, removing)`: the Creature add-to-level / remove routine (vtable entry).
const ADD_RVA: usize = 0x6bf60;
/// `mov [rsp+8],rbx; push rdi; sub rsp,0x40` (10 bytes, whole instructions).
const ADD_PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x40];

/// `0x1402672c0(template, &pos, 0, radio)`: spawns a Creature from a ship template node.
const SPAWN_RVA: usize = 0x2672c0;
/// `mov [rsp+0x10],rbx; mov [rsp+0x18],rsi` (10 bytes, rsp-relative, safe to replay first).
const SPAWN_PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18];
/// The pending level name, a game `std::string` (MSVC layout: buffer or pointer at +0, size at
/// +0x10, capacity at +0x18); written by the level loader `0x14025afd0` through `CopyString`.
const LEVEL_NAME: usize = 0x1_403c_1390;

/// Per-frame arcade battle manager `FUN_1402cc260` (called from the Combat scenario).
const BATTLE_RVA: usize = 0x2cc260;
/// `mov rax,rsp; push rdi; sub rsp,0xa0` (11 bytes; the next instruction reads RAX, which the hook preserves).
const BATTLE_PROLOGUE: [u8; 11] = [0x48, 0x8b, 0xc4, 0x57, 0x48, 0x81, 0xec, 0xa0, 0x00, 0x00, 0x00];

/// The AI target choice `FUN_140078b40(ship)` is called from the per-ship driver at two sites (`mov rcx,rbx; call`).
const TARGET_FN: usize = 0x1_4007_8b40;
const TARGET_SITES: [(usize, [u8; 5]); 2] = [
    (0x1_4007_3b93, [0xe8, 0xa8, 0x4f, 0x00, 0x00]),
    (0x1_4007_3c73, [0xe8, 0xc8, 0x4e, 0x00, 0x00]),
];

/// ammoSelectButtonPress? (0x14026f880): rebuilds the native ammo selector after a press.
const AMMO_PRESS: usize = 0x1_4026_f880;
const AMMO_PRESS_STOLEN: [u8; 10] =
    [0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10];

/// Vector of creatures the AI task runs (same entry layout): begin / end.
const AI_BEGIN: usize = 0x1_43fa_a608;
const AI_END: usize = 0x1_43fa_a610;
/// Side buckets (vectors of raw 8-byte pointers): begin / end.
const NEG_BEGIN: usize = 0x1_43fa_a5f0;
const NEG_END: usize = 0x1_43fa_a5f8;
const ZERO_BEGIN: usize = 0x1_43fa_a650;
const ZERO_END: usize = 0x1_43fa_a658;
const POS_BEGIN: usize = 0x1_43fa_a638;
const POS_END: usize = 0x1_43fa_a640;
/// Order index, `i32`, given by the Missile scenario.
const OFF_ORDER: usize = 0xc98;

static GAME_IS_STEAM_1_163: AtomicBool = AtomicBool::new(false);
const MAX_LINES: u32 = 600;
const DEBUG_MAX_LINES: u32 = 1_000_000;
static LINES: AtomicU32 = AtomicU32::new(0);
static SPAWN_LINES: AtomicU32 = AtomicU32::new(0);
static LAST_LEVEL: Mutex<String> = Mutex::new(String::new());
/// Signature of the last ADD/REMOVE line and how many identical ones were swallowed since.
static LAST_SIG: Mutex<(usize, u8, u32, u32, usize)> = Mutex::new((0, 0, 0, 0, 0));

/// The modloader may buffer its log: flush after the lines that matter for a crash report.
fn flush_log() {
    log::logger().flush();
}

/// A cheap pick in 0..n (no rand crate): the clock's nanoseconds.
fn pick(n: usize) -> usize {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.subsec_nanos() as usize / 7) % n
}

unsafe fn level_name() -> String {
    let capacity: usize = read(LEVEL_NAME + 0x18);
    let size: usize = read(LEVEL_NAME + 0x10);
    if size == 0 || size > capacity || size > 64 {
        return String::from("?");
    }
    let data = if capacity > 0xf { read::<usize>(LEVEL_NAME) } else { LEVEL_NAME };
    if data == 0 {
        return String::from("?");
    }
    String::from_utf8_lossy(std::slice::from_raw_parts(data as *const u8, size)).into_owned()
}

/// `sel=<scenario state> fl=<campaign><battle><pending>` for tagging lines when the name is unreadable.
unsafe fn scene() -> String {
    format!(
        "sel={} fl={}{}{}",
        read::<u32>(SCENE_SELECTOR),
        read::<u8>(SCENE_FLAGS[0]),
        read::<u8>(SCENE_FLAGS[1]),
        read::<u8>(SCENE_FLAGS[2])
    )
}

/// The `ALLY ADD`/`ALLY REMOVE` and `ALLY SPAWN` line budget. Debug mode removes the practical limit.
fn line_budget() -> u32 {
    if diag::debug() {
        DEBUG_MAX_LINES
    } else {
        MAX_LINES
    }
}

/// Returns the level tag for a new line, or `None` when this level's line budget is spent.
/// A changed level name resets the budget and writes a `LEVEL` line.
unsafe fn next_line() -> Option<String> {
    let level = level_name();
    {
        let mut last = LAST_LEVEL.lock().ok()?;
        if *last != level {
            *last = level.clone();
            LINES.store(0, Ordering::Relaxed);
            log::info!(
                "ALLY LEVEL {level} t={:.2} raw_size={} raw_cap={}",
                read::<f64>(CLOCK),
                read::<usize>(LEVEL_NAME + 0x10),
                read::<usize>(LEVEL_NAME + 0x18)
            );
        }
    }
    if LINES.fetch_add(1, Ordering::Relaxed) >= line_budget() {
        return None;
    }
    Some(level)
}


unsafe extern "C" fn on_creature(creature: *const u8, removing: u8) {
    let _hook = diag::scope("on_creature");
    if creature.is_null() {
        return;
    }
    let base = creature as usize;
    let sig = (
        base,
        removing,
        read::<f32>(base + OFF_SIDE).to_bits(),
        read::<f32>(base + OFF_AGGRESSION).to_bits(),
        count(CREATURES_BEGIN, CREATURES_END, 0x18),
    );
    if let Ok(mut last) = LAST_SIG.lock() {
        if (last.0, last.1, last.2, last.3, last.4) == sig {
            return; // the same creature added again with nothing changed: not worth a line
        }
        *last = sig;
    }
    let Some(level) = next_line() else { return };
    let controlled: usize = read(CONTROLLED_LINK + 8);
    log::info!(
        "ALLY {} level={} {} t={:.2} c={:#x} side={} ai_flag={} aggr={} state={} mode={} order={} target={:#x} controlled={} \
         | creatures={} ai={} neg={} zero={} pos={}",
        if removing == 0 { "ADD" } else { "REMOVE" },
        level,
        scene(),
        read::<f64>(CLOCK),
        base,
        read::<f32>(base + OFF_SIDE),
        read::<u8>(base + OFF_AI_FLAG),
        read::<f32>(base + OFF_AGGRESSION),
        read::<i32>(base + OFF_STATE),
        read::<i32>(base + OFF_MODE),
        read::<i32>(base + OFF_ORDER),
        read::<usize>(base + OFF_TARGET),
        controlled == base,
        count(CREATURES_BEGIN, CREATURES_END, 0x18),
        count(AI_BEGIN, AI_END, 0x18),
        count(NEG_BEGIN, NEG_END, 8),
        count(ZERO_BEGIN, ZERO_END, 8),
        count(POS_BEGIN, POS_END, 8),
    );
}


fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

unsafe extern "C" fn on_ammo_press() {
    let _hook = diag::scope("on_ammo_press");
    precombat::inspect_gui("ammoSelectButtonPress");
}

#[no_mangle]
unsafe extern "C" fn version(version: *const c_char) -> bool {
    let supported = CStr::from_ptr(version).to_bytes() == b"Steam 1.163";
    GAME_IS_STEAM_1_163.store(supported, Ordering::Relaxed);
    supported
}

#[no_mangle]
unsafe extern "C" fn init() -> bool {
    let path = config::PATH;
    let config: Config = config::load();
    let supported = GAME_IS_STEAM_1_163.load(Ordering::Relaxed);
    let image_ok = GetModuleHandleA(std::ptr::null()) as usize == IMAGE_BASE;
    // debug_log is the runtime switch; set it before install so the legacy cargo feature cannot force it
    diag::set_debug(supported && image_ok && config.enabled && config.logging.debug_log);
    // the first line of this mod in every log: testers must be able to show which build they run
    log::info!("{}", diag::banner());
    diag::install();
    if !supported {
        log::error!("hf_ally: needs Steam 1.163, not starting");
        return false;
    }
    if !image_ok {
        log::error!("hf_ally: unexpected image base, not starting");
        return false;
    }
    if !config.enabled {
        log::info!("hf_ally: disabled in {path}");
        return true;
    }
    if config.logging.debug_log {
        log::info!("hf_ally: debug_log on: megalog, watchdog and crash dump (large files, see ally.json)");
    }
    precombat::configure_marker(&config.marker_text);
    ally_ammo::set_enabled(config.ally_ai_ammo);
    missile_guard::set_autofire(config.ally_zenith_autofire);
    let honor_share = honor_share::set_share(config.ally_honor_share);
    if honor_share != config.ally_honor_share {
        log::warn!("hf_ally: ally_honor_share {} is outside 0..=1 or not finite; using {honor_share}", config.ally_honor_share);
    }
    log::info!(
        "hf_ally v{} config: {}",
        env!("CARGO_PKG_VERSION"),
        serde_json::to_string(&config).unwrap_or_default()
    );
    battle::configure(&config);

    match prepare(&config) {
        Ok(()) => {
            log::info!("hf_ally v{}: hooks installed (spawn log, creature log {})", env!("CARGO_PKG_VERSION"), config.logging.log_creatures);
            true
        }
        Err(error) => {
            log::error!("hf_ally: not installed: {error}");
            false
        }
    }
}

unsafe fn prepare(config: &Config) -> Result<(), String> {
    let module = ProcessModule::main().map_err(|e| e.to_string())?;
    let address = module.resolve_rva(ADD_RVA).map_err(|e| e.to_string())?;
    let found = std::slice::from_raw_parts(address as *const u8, ADD_PROLOGUE.len());
    if config.logging.log_creatures && found != ADD_PROLOGUE {
        return Err(format!(
            "bytes at {address:#x} are {found:02x?}, expected {ADD_PROLOGUE:02x?} (another mod owns this site, or a different build)"
        ));
    }
    let spawn = module.resolve_rva(SPAWN_RVA).map_err(|e| e.to_string())?;
    let found = std::slice::from_raw_parts(spawn as *const u8, SPAWN_PROLOGUE.len());
    if found != SPAWN_PROLOGUE {
        return Err(format!(
            "bytes at {spawn:#x} are {found:02x?}, expected {SPAWN_PROLOGUE:02x?} (another mod owns this site, or a different build)"
        ));
    }

    if sound::verify() {
        sound::configure(config.ally_sounds, true);
    }

    let mut session = PatchSession::new();
    // At the function entry RSP is 8 mod 16 and [RSP] is the return address. Replay the two
    // rsp-relative stores, pad 8 bytes to align, then call with the return address (now at
    // RSP+0xC8 behind the helper's 7 pushes and 0x88 bytes) in RDX; RCX is still the template.
    let mut trampoline = Trampoline::new();
    trampoline.bytes(&SPAWN_PROLOGUE);
    trampoline.bytes(&[0x48, 0x83, 0xec, 0x08]); // sub rsp, 8
    trampoline.preserved_call(
        battle::on_spawn as *const (),
        &[0x48, 0x8b, 0x94, 0x24, 0xc8, 0x00, 0x00, 0x00], // mov rdx, [rsp+0xC8]
        &[],
        ReturnType::None,
    );
    trampoline.bytes(&[0x48, 0x83, 0xc4, 0x08]); // add rsp, 8
    trampoline.absolute_jump(spawn + SPAWN_PROLOGUE.len());
    session
        .detour_trampoline(spawn, SPAWN_PROLOGUE.len(), trampoline)
        .map_err(|e| e.to_string())?;
    if config.logging.log_creatures {
        session
            .patch_call(
                address,
                on_creature as *const (),
                ADD_PROLOGUE.len(),
                true,
                ReturnType::None,
            )
            .map_err(|e| e.to_string())?;
    }
    // The pre-combat ALLY and plane-loadout buttons are part of the mod; install them whenever it is enabled.
    let sites: [(usize, [u8; 5], *const ()); 3] = [
        (
            precombat::PRECOMBAT_SEAM,
            precombat::PRECOMBAT_SEAM_BYTES,
            precombat::on_precombat as *const (),
        ),
        (
            precombat::PRECOMBAT_B1_SEAM,
            precombat::PRECOMBAT_B1_BYTES,
            precombat::on_precombat_b1 as *const (),
        ),
        (
            precombat::PRECOMBAT_B2_SEAM,
            precombat::PRECOMBAT_B2_BYTES,
            precombat::on_precombat_b2 as *const (),
        ),
    ];
    for (site, bytes, hook) in sites {
        let found = std::slice::from_raw_parts(site as *const u8, bytes.len());
        if found != bytes {
            return Err(format!("pre-combat seam {site:#x} has unexpected bytes {found:02x?}"));
        }
        let mut trampoline = Trampoline::new();
        trampoline.preserved_call(hook, &[], &[], ReturnType::None);
        trampoline.absolute_call(precombat::PRECOMBAT_SEAM_TARGET as *const ());
        trampoline.relative_jump(site + bytes.len());
        session.detour_trampoline(site, bytes.len(), trampoline).map_err(|e| e.to_string())?;
    }
    for (name, address, expected) in [
        ("gui new", precombat::GUI_NEW, &precombat::GUI_NEW_PROLOGUE[..]),
        ("gui ctor", precombat::GUI_CTOR, &precombat::GUI_CTOR_PROLOGUE[..]),
        ("gui setpos", precombat::GUI_SETPOS, &precombat::GUI_SETPOS_PROLOGUE[..]),
        ("gui setbg", precombat::GUI_SETBG, &precombat::GUI_SETBG_PROLOGUE[..]),
        (
            "gui sprite name",
            precombat::SET_SPRITE_NAME,
            &precombat::SET_SPRITE_NAME_PROLOGUE[..],
        ),
        (
            "gui sprite clear",
            precombat::OBJECT_ARRAY_DELETE_ALL,
            &precombat::OBJECT_ARRAY_DELETE_ALL_PROLOGUE[..],
        ),
        ("gui reset", precombat::GUI_RESET, &precombat::GUI_RESET_PROLOGUE[..]),
        ("gui find", precombat::GUI_FIND, &precombat::GUI_FIND_PROLOGUE[..]),
    ] {
        let found = std::slice::from_raw_parts(address as *const u8, expected.len());
        if found != expected {
            return Err(format!("{name} at {address:#x} has unexpected bytes {found:02x?}: toggle not installed"));
        }
    }
    let found = std::slice::from_raw_parts(precombat::CLICK_ENTRY as *const u8, precombat::CLICK_STOLEN.len());
    if found != precombat::CLICK_STOLEN {
        log::warn!("hf_ally: ClickFunc entry is already hooked by another mod (hf_probe hook_clicks?): no toggle, no GUI inspector");
    } else {
        // entry hook that can swallow the click: call the hook with the four arguments, `ret` when it says yes
        let mut code: Vec<u8> = vec![0x51, 0x52, 0x41, 0x50, 0x41, 0x51, 0x48, 0x83, 0xec, 0x28, 0x48, 0xb8];
        code.extend((precombat::on_click_pre as *const () as usize as u64).to_le_bytes());
        code.extend([0xff, 0xd0, 0x48, 0x83, 0xc4, 0x28, 0x84, 0xc0, 0x41, 0x59, 0x41, 0x58, 0x5a, 0x59]);
        code.extend([0x75, (precombat::CLICK_STOLEN.len() + 14) as u8]);
        code.extend(precombat::CLICK_STOLEN);
        code.extend([0xff, 0x25, 0, 0, 0, 0]);
        code.extend(((precombat::CLICK_ENTRY + precombat::CLICK_STOLEN.len()) as u64).to_le_bytes());
        code.push(0xc3);
        let mut trampoline = Trampoline::new();
        trampoline.bytes(&code);
        session
            .detour_trampoline(precombat::CLICK_ENTRY, precombat::CLICK_STOLEN.len(), trampoline)
            .map_err(|e| e.to_string())?;
        precombat::set_ui_ready();
    }
    let bytes = std::slice::from_raw_parts(precombat::MARKER_SEAM as *const u8, 5);
    let target = (precombat::MARKER_SEAM + 5).wrapping_add(i32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as isize as usize);
    if bytes[0] == 0xe8 && target == precombat::FLUSH {
        let mut trampoline = Trampoline::new();
        trampoline.absolute_call(precombat::FLUSH as *const ());
        trampoline.preserved_call(precombat::draw_marker as *const (), &[], &[], ReturnType::None);
        trampoline.relative_jump(precombat::MARKER_SEAM + 5);
        session.detour_trampoline(precombat::MARKER_SEAM, 5, trampoline).map_err(|e| e.to_string())?;
    } else {
        log::warn!("hf_ally: render seam {:#x} is not a call to the flush: not hooked", precombat::MARKER_SEAM);
    }
    // per-frame input task (game thread): re-places the buttons after the cards are shuffled. Another mod on the
    // same entry (hf_voice) makes the bytes differ: then nothing ticks on this screen (the battle manager hook
    // does not run on it, seen in the v0.11.4-0.11.8 logs) and the buttons stay where the build put them.
    let found = std::slice::from_raw_parts(precombat::INPUT_TASK as *const u8, precombat::INPUT_TASK_PROLOGUE.len());
    if found == precombat::INPUT_TASK_PROLOGUE {
        // padded like hf_voice's: at a function entry RSP is 8 mod 16, patchy's register saver wants 0
        let mut trampoline = Trampoline::new();
        trampoline.bytes(&[0x48, 0x83, 0xec, 0x08]);
        trampoline.preserved_call(precombat::on_input_frame as *const (), &[], &[], ReturnType::None);
        trampoline.bytes(&[0x48, 0x83, 0xc4, 0x08]);
        trampoline.bytes(&precombat::INPUT_TASK_PROLOGUE);
        trampoline.relative_jump(precombat::INPUT_TASK + precombat::INPUT_TASK_PROLOGUE.len());
        session
            .detour_trampoline(precombat::INPUT_TASK, precombat::INPUT_TASK_PROLOGUE.len(), trampoline)
            .map_err(|e| e.to_string())?;
    } else {
        log::warn!("hf_ally: SomethingInputTask entry differs (another mod hooked it?): no per-frame card check from it");
    }
    precombat::configure_inspect(config.logging.gui_inspect);
    if config.logging.gui_inspect {
        // ammoSelectButtonPress? (the native ammo selector rebuild): log-only entry hook, 10 stolen rsp-relative stores
        let found = std::slice::from_raw_parts(AMMO_PRESS as *const u8, AMMO_PRESS_STOLEN.len());
        if found == AMMO_PRESS_STOLEN {
            let mut trampoline = Trampoline::new();
            trampoline.bytes(&[0x48, 0x83, 0xec, 0x08]);
            trampoline.preserved_call(on_ammo_press as *const (), &[], &[], ReturnType::None);
            trampoline.bytes(&[0x48, 0x83, 0xc4, 0x08]);
            trampoline.bytes(&AMMO_PRESS_STOLEN);
            trampoline.relative_jump(AMMO_PRESS + AMMO_PRESS_STOLEN.len());
            session
                .detour_trampoline(AMMO_PRESS, AMMO_PRESS_STOLEN.len(), trampoline)
                .map_err(|e| e.to_string())?;
        } else {
            log::warn!("hf_ally: ammoSelectButtonPress? bytes differ, not hooked");
        }
    }
    if config.ally_ai_ammo {
        let found = std::slice::from_raw_parts(ally_ammo::REFUND as *const u8, ally_ammo::REFUND_STOLEN.len());
        if found == ally_ammo::REFUND_STOLEN {
            let mut trampoline = Trampoline::new();
            trampoline.bytes(&[0x48, 0x83, 0xec, 0x08]);
            trampoline.preserved_call(ally_ammo::on_refund as *const (), &[], &[], ReturnType::None);
            trampoline.bytes(&[0x48, 0x83, 0xc4, 0x08]);
            trampoline.bytes(&ally_ammo::REFUND_STOLEN);
            trampoline.relative_jump(ally_ammo::REFUND + ally_ammo::REFUND_STOLEN.len());
            session
                .detour_trampoline(ally_ammo::REFUND, ally_ammo::REFUND_STOLEN.len(), trampoline)
                .map_err(|e| e.to_string())?;
        } else {
            log::warn!("hf_ally: refund function bytes differ ({found:02x?}), not hooked: allies on AP / proximity would credit free ammo");
            ally_ammo::set_enabled(false);
        }
    }
    battle::set_spare_prizes(config.spare_prizes);
    if config.spare_prizes {
        let target_ok = std::slice::from_raw_parts(TARGET_FN as *const u8, 3) == [0x48, 0x8b, 0xc4];
        let sites_ok = TARGET_SITES.iter().all(|(site, bytes)| std::slice::from_raw_parts(*site as *const u8, 5) == bytes);
        if target_ok && sites_ok {
            for (site, bytes) in TARGET_SITES {
                let mut trampoline = Trampoline::new();
                trampoline.bytes(&[0x51, 0x48, 0x83, 0xec, 0x28]); // push rcx; sub rsp, 0x28
                trampoline.absolute_call(TARGET_FN as *const ());
                trampoline.bytes(&[0x48, 0x83, 0xc4, 0x28, 0x59]); // add rsp, 0x28; pop rcx
                trampoline.preserved_call(battle::on_target_chosen as *const (), &[], &[], ReturnType::None);
                trampoline.relative_jump(site + bytes.len());
                session.detour_trampoline(site, bytes.len(), trampoline).map_err(|e| e.to_string())?;
            }
        } else {
            log::warn!("hf_ally: the AI target choice or its call sites differ, prize ships are not spared");
            battle::set_spare_prizes(false);
        }
    }
    let guard_missiles = config.ally_bombers || !config.ally_zenith_autofire;
    if guard_missiles {
        missile_guard::install(&mut session)?;
    }
    if config.ally_bombers {
        bomber::install(&mut session)?;
    }
    // the filter applies to every AI ship, the enemy's included
    if let Err(error) = intercept::install(&mut session, &config.intercept_ignore_oids) {
        log::warn!("hf_ally: {error}");
    }
    enemy_waves::install(&mut session, config.enemy_group_size);
    if config.plane_loadouts {
        match plane_loadouts::install(&mut session) {
            Ok(()) => {
                precombat::set_plane_buttons(true);
                plane_loadouts::install_aa(&mut session);
                plane_loadouts::install_no_rocket_draw(&mut session);
                plane_loadouts::install_test_stock(&mut session);
                plane_loadouts::install_aa_limits(&mut session, config.plane_aa_range, config.plane_aa_angle);
                plane_loadouts::AA_DELAY_S.store(config.plane_aa_delay.clamp(0.0, 60.0).to_bits(), Ordering::Relaxed);
                plane_loadouts::AA_SPEED.store(config.plane_aa_speed.to_bits(), Ordering::Relaxed);
            }
            Err(error) => log::warn!("hf_ally: {error}"),
        }
    }
    plane_loadouts::ATTACK_DELAY_S.store(config.plane_attack_delay.clamp(0.0, 120.0).to_bits(), Ordering::Relaxed);
    let ally_damage_color = match parse_hex_color(&config.ally_damage_color) {
        Some(color) => color,
        None => {
            log::warn!(
                "hf_ally: ally_damage_color {:?} is not HEX (want \"#RRGGBB\"); using #00E4FF",
                config.ally_damage_color
            );
            0x00e4ff
        }
    };
    ally_hud::install(&mut session, ally_damage_color);
    if config.ally_panels {
        ally_panels::install(&mut session);
    }
    let reticle_scale = config.ally_reticle_scale;
    if reticle_scale != 0.0 || config.debug_overlay.enable {
        // the reticle module owns the single per-frame battle HUD post-pass that both features draw from
        ally_reticles::install(&mut session, ally_damage_color, reticle_scale, &config.debug_overlay);
    }
    plane_loadouts::EXTRA_GUNPOD.store(config.extra_gunpod, Ordering::Relaxed);
    plane_loadouts::K13_ON.store(config.plane_k13, Ordering::Relaxed);
    // Wingman deployment and battle-manager hooks; always installed when the mod is enabled.
    for (name, address, expected) in [
        ("deploy", battle::DEPLOY, &battle::DEPLOY_PROLOGUE[..]),
        ("set_controlled", SET_CONTROLLED, &SET_CONTROLLED_PROLOGUE[..]),
        ("set_link", SET_LINK, &SET_LINK_PROLOGUE[..]),
        ("get_profile", GET_PROFILE, &GET_PROFILE_PROLOGUE[..]),
        ("extract", battle::EXTRACT, &battle::EXTRACT_PROLOGUE[..]),
    ] {
        let found = std::slice::from_raw_parts(address as *const u8, expected.len());
        if found != expected {
            return Err(format!("{name} at {address:#x} has unexpected bytes {found:02x?}"));
        }
    }
    let battle = module.resolve_rva(BATTLE_RVA).map_err(|e| e.to_string())?;
    let found = std::slice::from_raw_parts(battle as *const u8, BATTLE_PROLOGUE.len());
    if found != BATTLE_PROLOGUE {
        return Err(format!("battle manager at {battle:#x} has unexpected bytes {found:02x?}"));
    }
    if config.ship_switching {
        ship_switch::verify_native()?;
    }
    ship_switch::ENABLED.store(config.ship_switching, Ordering::Relaxed);
    session
        .patch_call(
            battle,
            battle::on_battle_tick as *const (),
            BATTLE_PROLOGUE.len(),
            true,
            ReturnType::None,
        )
        .map_err(|e| e.to_string())?;
    if honor_share::share() > 0.0 {
        honor_share::verify_native()?;
        // SAFETY: the 1.163 site bytes were just checked; this is mod initialization.
        unsafe { honor_share::install(&mut session)? };
        log::info!("hf_ally: ally honor sharing at {:.0}%", honor_share::share() * 100.0);
    }
    session.install_permanently().map_err(|e| e.to_string())
}
