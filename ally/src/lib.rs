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
mod ship_switch;
mod ally_reticles;
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
mod native_layout;
mod plane_loadouts;
mod precombat;

use std::{
    ffi::{c_char, CStr},
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Mutex,
    },
};

use std::mem::{offset_of, size_of};

use native_layout::{Body, Creature, Link, Node};

use config::{parse_hex_color, Config};
use patchy::{PatchSession, ProcessModule, ReturnType, Trampoline};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleA;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

const IMAGE_BASE: usize = 0x1_4000_0000;

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
/// Template node: class code (u64).
const OFF_CODE: usize = native_layout::NODE_CODE;

/// Per-frame arcade battle manager `FUN_1402cc260` (called from the Combat scenario).
const BATTLE_RVA: usize = 0x2cc260;
/// `mov rax,rsp; push rdi; sub rsp,0xa0` (11 bytes; the next instruction reads RAX, which the hook preserves).
const BATTLE_PROLOGUE: [u8; 11] = [0x48, 0x8b, 0xc4, 0x57, 0x48, 0x81, 0xec, 0xa0, 0x00, 0x00, 0x00];
/// Game functions the ally calls, with a prologue check each.
const DEPLOY: usize = 0x1_402d_6e90; // deploys the front of the reserve (named rammingCheckCollision__ in Ghidra)
const DEPLOY_PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18];
const SET_CONTROLLED: usize = 0x1_402a_7e30; // (profile, ship, slot): AI flag := not controlled
const SET_CONTROLLED_PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20];
const SET_LINK: usize = 0x1_4019_4de0; // (link, node)
const SET_LINK_PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20];
const GET_PROFILE: usize = 0x1_4029_2a90;
const GET_PROFILE_PROLOGUE: [u8; 8] = [0x0f, 0xb6, 0x15, 0xea, 0x44, 0xd3, 0x07, 0x0f];
/// Reserve list of ships waiting to deploy (`std::list`: sentinel at +0, size at +8) and the roster.
const EXTRACT: usize = 0x1_4020_ef30; // (creature, flag): hands a living ship back to the fleet, removes it from the level
const EXTRACT_PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18];
const GUI_ROOT: usize = 0x1_47fc_6f88; // pointer to the global node every GUI is added to
/// The AI target choice `FUN_140078b40(ship)` is called from the per-ship driver at two sites (`mov rcx,rbx; call`).
const TARGET_FN: usize = 0x1_4007_8b40;
const TARGET_SITES: [(usize, [u8; 5]); 2] = [
    (0x1_4007_3b93, [0xe8, 0xa8, 0x4f, 0x00, 0x00]),
    (0x1_4007_3c73, [0xe8, 0xc8, 0x4e, 0x00, 0x00]),
];
const OFF_PRIZE: usize = 0xc19; // u8: the ship may be captured after the battle (ship names Pollux, Kadesh Mk1, ...)
const OFF_TARGET_LINK: usize = offset_of!(Creature, target); // the Link whose node is the target at +0xb00
const RESERVE: usize = 0x1_47fc_73b0;
const ROSTER_BEGIN: usize = 0x1_47fc_7590;
const ROSTER_END: usize = 0x1_47fc_7598;
/// Arena bounds and constants the manager passes to the deploy function.
const ARENA_X_HIGH: usize = 0x1_403c_2bc0; // f32
const ARENA_X_LOW: usize = 0x1_403c_2bc8; // f32
const ARENA_Y_LOW: usize = 0x1_403c_2bcc; // f32
const DEPLOY_K: usize = 0x1_4035_b974; // f32, -300.0
const DEPLOY_AGGR: usize = 0x1_4035_24d8; // f32, 1.0
/// Controlled-ship links, one per control slot (0x18 bytes each).
const CONTROLLED_LINKS: usize = 0x1_43a1_c260;
const ALLY_SLOT: usize = 1;
const OFF_MODE_D70: usize = offset_of!(Creature, order_role); // i32: 1 = flown by the player, 2 = wingman
const OFF_ACTIVE: usize = offset_of!(Node, state); // i32: non-zero while the ship is in the level

/// Byte stride of one `Link` entry in the native object vectors below: the pointer sits at +8.
const LINK_STRIDE: usize = size_of::<Link>();
/// Ceiling on the entries read from one battle roster / ship part / gun list. A real ship has far fewer, so a
/// larger count means the native vector header is corrupt.
const SHIP_LIST_LIMIT: usize = 64;
/// Ceiling on the entries read from the global creature list, for the same reason.
const CREATURE_LIST_LIMIT: usize = 4096;
/// Global creature list (`Link` entries, 0x18 bytes, pointer at +8): begin / end.
const CREATURES_BEGIN: usize = 0x1_43fa_a5d8;
const CREATURES_END: usize = 0x1_43fa_a5e0;
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
/// Smart-ref `Link` to the creature the player controls (node pointer at +8).
const CONTROLLED_LINK: usize = 0x1_43a1_c260;
/// Game clock, `f64` seconds.
const CLOCK: usize = 0x1_43a1_19c0;

const OFF_SIDE: usize = offset_of!(Creature, side); // f32: negative = AI, positive = player, 0 = neutral
const OFF_AI_FLAG: usize = offset_of!(Creature, ai_flag); // u8: non-zero = joins the AI vector when added
const OFF_AGGRESSION: usize = 0xbc4; // f32: >= 0.5 makes the driver call FUN_140079b20(ship, 1)
const OFF_STATE: usize = offset_of!(Creature, driver_state); // i32: driver state (1/2 run the AI tick)
const OFF_MODE: usize = offset_of!(Creature, mode); // i32
const OFF_ORDER: usize = 0xc98; // i32: index given by the Missile scenario
const OFF_TARGET: usize = offset_of!(Creature, target) + offset_of!(Link, target); // pointer: current target (link)

static GAME_IS_STEAM_1_163: AtomicBool = AtomicBool::new(false);
const MAX_LINES: u32 = 600;
const DEBUG_MAX_LINES: u32 = 1_000_000;
static LINES: AtomicU32 = AtomicU32::new(0);
static SPAWN_LINES: AtomicU32 = AtomicU32::new(0);
/// The ship mode (`+0xd70`) an ally gets: 0 = the plain AI ship the enemies use. It was the config key `ally_mode`
/// until v0.12.1; 1 (flown by a player) makes an ally look like the flagship and 2 (strike run) is the retreat order,
/// so only 0 is valid for an ally (hf_notes docs/findings/battle-ally.md, "ally_mode removed").
const ALLY_MODE: i32 = 0;
static ALLY_DONE: AtomicBool = AtomicBool::new(false);
static ARMED: AtomicBool = AtomicBool::new(false);

/// The modloader may buffer its log: flush after the lines that matter for a crash report.
fn flush_log() {
    log::logger().flush();
}
/// The ally being followed in the log (0 = none), the game time of its next track line and of the last one.
static TRACK_ALLY: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static TRACK_NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static TRACK_END: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// The player's deploy call site (`0x1402d6f7a` + 5) and the scenario state in which a battle starts.
const PLAYER_DEPLOY_CALLER_RVA: usize = 0x2d6f7f;
const BATTLE_START_SELECTOR: u32 = 1;
const OFF_POS_X: usize = offset_of!(Creature, bounds_center_x); // f32 creature centre
const OFF_POS_Y: usize = offset_of!(Creature, bounds_center_y);
const OFF_WAYPOINT_X: usize = offset_of!(Creature, waypoint_x); // f32 steering waypoint written by the AI tick
const OFF_WAYPOINT_Y: usize = offset_of!(Creature, waypoint_y);
// --- probe fields (all [code]/guess, see the findings note) ---
const OFF_RETARGET: usize = offset_of!(Creature, retarget_timer); // f32 seconds until the next target re-roll
const OFF_FLAGS: usize = offset_of!(Creature, trigger_gun_group_0); // 5 control bytes: group 0, group 1, bombs, spare, flares
const OFF_LAUNCH_CD: usize = 0xd60; // f32 ship launch cooldown
const OFF_LAUNCH_COUNT: usize = 0xc98; // i32 per-ship launch counter
const OFF_ZENITH: (usize, usize) = (
    offset_of!(Creature, anti_ship_launchers),
    offset_of!(Creature, anti_ship_launchers) + size_of::<usize>(),
); // anti-ship launcher list
const OFF_GUNS: (usize, usize) = (
    offset_of!(Creature, ammo_consumers),
    offset_of!(Creature, ammo_consumers) + size_of::<usize>(),
); // ammo-consumer list, the guns and launchers that draw rounds
const OFF_PART_COOLDOWN: usize = 0x848; // f32; highfleet-rs models this word as Body::mdl_covered
const OFF_PART_RESERVE: usize = offset_of!(Body, weapon_ammo_amount); // i32
const OFF_PART_LOADED: usize = offset_of!(Body, weapon_ammo_loaded); // f32
/// Game time (f64 bits) at which the player's ship was first seen in this battle; 0 = not yet.
static HANDOVER: AtomicBool = AtomicBool::new(false);
/// The retreat point (x, y as two f32 packed in a u64) the game keeps at 0x147fc7638; zeroed when a ship uses it.
const RETREAT_POINT: usize = 0x1_47fc_7638;
static RETREAT_POINT_SAVED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static RETREAT_KEY: AtomicU32 = AtomicU32::new(0);
static KEY_WAS_DOWN: AtomicBool = AtomicBool::new(false);
/// The ally of this battle (stays set until the next battle), the flagship last seen and whether it was alive.
static ALLY_PTR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static FLAGSHIP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static FLAGSHIP_ALIVE: AtomicBool = AtomicBool::new(false);
static FLAGSHIP_STATE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
static PLAYER_SEEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static ALLY_DELAY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static LAST_LEVEL: Mutex<String> = Mutex::new(String::new());
/// Signature of the last ADD/REMOVE line and how many identical ones were swallowed since.
static LAST_SIG: Mutex<(usize, u8, u32, u32, usize)> = Mutex::new((0, 0, 0, 0, 0));
/// Scene flags (see hf_probe): campaign / battle bytes, and the scenario state selector.
const SCENE_SELECTOR: usize = 0x1_47fc_6fc4; // u32
const SCENE_FLAGS: [usize; 3] = [0x1_47fc_6fd0, 0x1_47fc_6fda, 0x1_43a1_1902];

// --- sounds: radio lines (v0.10.0) and pre-combat button clicks ---
// `playSFXSound_` (0x1402e30e0) is what the stock battle radio (`PlayRadioSounds_`, 0x1402d5910) ends in; the call
// below repeats its last call exactly: kind 0xb, the sound resource name from `Media/Snd/sound.res`, the global GUI
// node, a base volume, then the stack arguments 1.0, a status float it writes into, and zeros. The stock radio also
// appends `_eng` to its names when the game is in English, so `play_radio` does the same (v0.14.26).
const PLAY_SFX: usize = 0x1_402e_30e0;
const PLAY_SFX_PROLOGUE: [u8; 15] = [0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x48, 0x89, 0x68, 0x10, 0x48, 0x89, 0x70, 0x18];
/// Kind 0xb: the stock radio and the ClickFunc UI sounds all play through it.
const SFX_KIND_STOCK: i32 = 0xb;
type PlaySfxFn = unsafe extern "C" fn(i32, *const u8, usize, f32, f32, *mut f32, u64, u8, i32, u8, u8, f32, u8) -> f32;
/// `coreLanguage`: 1 = Russian, 2 = English. The options language button writes it live (`ClickFunc`
/// 0x140131af8), and the stock radio appends `_eng` exactly when it is 2 (`String_AppendEngSuffix`
/// 0x1401b20f0). Read it per line so a live language change applies without a restart.
const CORE_LANGUAGE: usize = 0x1_403c_0e1c;
const LANGUAGE_ENGLISH: i32 = 2;
/// `sound.res` maps the plain names to `Media/Snd/RADIO` (Russian) and the `_eng` names to
/// `Media/Snd/RADIO_ENG` (English); every line the mod plays has an `_eng` variant.
const ENG_SUFFIX: &[u8] = b"_eng";
/// Ally radio lines: the retreat key, an ally joining the fight, an ally destroyed.
static RADIO_SOUNDS: AtomicBool = AtomicBool::new(false);
/// Pre-combat ALLY and plane-loadout button clicks (independent of `ally_sounds`).
static BUTTON_SOUNDS: AtomicBool = AtomicBool::new(false);
/// The game keeps the pointer it is given (sound object +0x88) and, every frame, multiplies the sound's volume by
/// the float behind it (the stock radio stores the call's result, 1.0, there): 1.0f here, or the sound is silent.
static SFX_STATUS: [AtomicU32; 8] = [const { AtomicU32::new(0x3f80_0000) }; 8];
static SFX_NEXT: AtomicU32 = AtomicU32::new(0);
/// Allies already announced as destroyed (pointer reuse: entries are dropped when the ship leaves the roster).
static ANNOUNCED_LOST: Mutex<Vec<usize>> = Mutex::new(Vec::new());

const SND_DANGER: &[u8] = b"mission_danger_01\0";
const SND_LOST: [&[u8]; 2] = [b"radio_stall_01\0", b"radio_lost_energy_01\0"];
const SND_RETREATED: [&[u8]; 3] = [b"mission_end_01\0", b"mission_end_02\0", b"mission_end_03\0"];
const SND_JOIN: [&[u8]; 3] = [b"mission_start_01\0", b"mission_start_02\0", b"mission_start_03\0"];
/// Stock `Media/Snd/button_01.wav`, named `button_01` in `sound.res`. The game itself plays it as a UI click in
/// `Map_BuildMissileMarkers` (`0x1401c4ee0`), with kind 0xb and base volume 30.0.
const SND_BUTTON: &[u8] = b"button_01\0";
const BUTTON_VOLUME: f32 = 30.0;

/// Plays one sound by its `sound.res` name (NUL-terminated); `what` is for the log.
unsafe fn play_sfx(name: &[u8], volume: f32, what: &str) {
    let node: usize = read(GUI_ROOT);
    if node == 0 {
        log::info!("ALLY sound ({what}) skipped: no global node yet");
        return;
    }
    let slot = &SFX_STATUS[SFX_NEXT.fetch_add(1, Ordering::Relaxed) as usize % SFX_STATUS.len()];
    slot.store(0x3f80_0000, Ordering::Relaxed);
    let play: PlaySfxFn = std::mem::transmute(PLAY_SFX);
    let result = play(SFX_KIND_STOCK, name.as_ptr(), node, volume, 1.0, slot.as_ptr() as *mut f32, 0, 0, 0, 0, 0, 0.0, 1);
    log::info!("ALLY sound ({what}) {} -> {result}", String::from_utf8_lossy(&name[..name.len() - 1]));
}

/// Appends `_eng` to a stock radio name in English, matching `String_AppendEngSuffix` and the
/// `sound.res` entries that point at `Media/Snd/RADIO_ENG`. Returns a NUL-terminated name.
unsafe fn localized_radio_name(name: &[u8]) -> Vec<u8> {
    let base = name.strip_suffix(&[0]).unwrap_or(name);
    let mut localized = Vec::with_capacity(base.len() + ENG_SUFFIX.len() + 1);
    localized.extend_from_slice(base);
    if read::<i32>(CORE_LANGUAGE) == LANGUAGE_ENGLISH {
        localized.extend_from_slice(ENG_SUFFIX);
    }
    localized.push(0);
    localized
}

/// Plays a radio sound by its `sound.res` name, in the player's language.
unsafe fn play_radio(name: &[u8], what: &str) {
    if !RADIO_SOUNDS.load(Ordering::Relaxed) {
        return;
    }
    let localized = localized_radio_name(name);
    play_sfx(&localized, 60.0, what);
}

/// Plays the stock UI click for one of our pre-combat buttons.
unsafe fn play_button_sound(what: &str) {
    if BUTTON_SOUNDS.load(Ordering::Relaxed) {
        play_sfx(SND_BUTTON, BUTTON_VOLUME, what);
    }
}

/// A cheap pick in 0..n (no rand crate): the clock's nanoseconds.
fn pick(n: usize) -> usize {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.subsec_nanos() as usize / 7) % n
}

/// Last state seen per ally still in the roster (a destroyed ship is not flagged 6/7: it just leaves the roster).
static ALLY_LAST: Mutex<Vec<(usize, i32)>> = Mutex::new(Vec::new());

/// Announces allies that have just been destroyed; runs every battle frame. The game removes a destroyed ship from the
/// roster with its state still 1 (v0.10.1 test), so an ally that is gone counts as destroyed unless it was
/// retreating (state 2) or extracted, or no enemy is left (victory tears the roster down). A ship seen in state 6/7
/// still in the roster counts too.
unsafe fn ally_sound_events(ships: &[usize]) {
    let (Ok(mut announced), Ok(mut allies), Ok(mut last)) = (ANNOUNCED_LOST.lock(), ALLIES.lock(), ALLY_LAST.lock()) else { return };
    announced.retain(|p| ships.contains(p));
    for &p in allies.iter().filter(|p| ships.contains(p)) {
        let state = read::<i32>(p + OFF_STATE);
        match last.iter_mut().find(|e| e.0 == p) {
            Some(e) => e.1 = state,
            None => last.push((p, state)),
        }
    }
    let enemies = count(ENEMIES_BEGIN, ENEMIES_END, 0x18);
    let mut gone = Vec::new();
    for &p in allies.iter() {
        let here = ships.contains(&p);
        let last_state = last.iter().find(|e| e.0 == p).map_or(-1, |e| e.1);
        let dead = if here {
            matches!(read::<i32>(p + OFF_STATE), 6 | 7)
        } else {
            gone.push(p);
            let extracted = SUM_FLAGS.load(Ordering::Relaxed) & F_EXTRACTED != 0 || matches!(last_state, 2 | 6 | 7 | -1);
            if extracted || enemies == 0 {
                log::info!("ALLY c={p:#x} left the roster, not destroyed (last state {last_state}, enemies {enemies})");
            }
            // retreated alive: flying out (state 2) when it vanished, or extracted with the flagship
            if SUM_FLAGS.load(Ordering::Relaxed) & F_EXTRACTED != 0 || last_state == 2 {
                play_radio(SND_RETREATED[pick(SND_RETREATED.len())], "ally retreated");
            }
            !extracted && enemies != 0
        };
        if !dead || announced.contains(&p) {
            continue;
        }
        announced.push(p);
        SUM_LOST.fetch_add(1, Ordering::Relaxed);
        if ALLY_PTR.load(Ordering::Relaxed) == p {
            SUM_ALLY_STATE.store(6, Ordering::Relaxed);
        }
        log::info!("ALLY lost: c={p:#x} destroyed (last state {last_state}, in roster {here}, enemies {enemies})");
        flush_log();
        play_radio(SND_LOST[pick(SND_LOST.len())], "ally destroyed");
    }
    allies.retain(|p| !gone.contains(p));
    last.retain(|e| ships.contains(&e.0));
}

unsafe fn read<T: Copy>(address: usize) -> T {
    (address as *const T).read_unaligned()
}

/// Number of `stride`-byte entries between two vector pointers stored at `begin` / `end`.
unsafe fn count(begin: usize, end: usize, stride: usize) -> usize {
    let (b, e): (usize, usize) = (read(begin), read(end));
    if e >= b {
        (e - b) / stride
    } else {
        0
    }
}

/// The pending level name, read defensively.
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

/// Called at the start of `0x1402672c0` with the template and the caller's return address.
unsafe extern "C" fn on_spawn(template: usize, caller: usize) {
    let _hook = diag::scope("on_spawn");
    // a new battle begins when the player's ship is deployed at the start (selector 1); the ally's own
    // deploy goes through the same call site later (selector 2) and must not re-arm
    if caller.wrapping_sub(IMAGE_BASE) == PLAYER_DEPLOY_CALLER_RVA && read::<u32>(SCENE_SELECTOR) == BATTLE_START_SELECTOR {
        restore_orders("next battle", true);
        ALLY_DONE.store(false, Ordering::Relaxed);
        PLAYER_SEEN.store(0, Ordering::Relaxed);
        TRACK_ALLY.store(0, Ordering::Relaxed);
        bomber::reset();
        ally_panels::invalidate();
        // Probe pointers belong to one battle only. A restart can reuse a freed ship's
        // address for a GUI object whose active field also happens to be nonzero.
        if let (Ok(mut ally_probe), Ok(mut enemy_probe)) = (PROBE_ALLY.lock(), PROBE_ENEMY.lock()) {
            *ally_probe = Probe::default();
            *enemy_probe = Probe::default();
        }
        SPAWN_LINES.store(0, Ordering::Relaxed);
        PRIZE_LINES.store(0, Ordering::Relaxed);
        let armed = precombat::take_seen();
        ARMED.store(armed, Ordering::Relaxed);
        log::info!(
            "ALLY battle start: scene selector {}, {}",
            read::<u32>(SCENE_SELECTOR),
            if armed { "armed (entered through the pre-combat screen)" } else { "NOT armed (no pre-combat screen: re-engaged chase or other battle), the mod stays out of it" }
        );
        flush_log();
        emit_summary(f64::from_bits(SUM_LAST.load(Ordering::Relaxed)));
        ALLY_PTR.store(0, Ordering::Relaxed);
        FLAGSHIP.store(0, Ordering::Relaxed);
        RETREAT_POINT_SAVED.store(0, Ordering::Relaxed);
        if let Ok(mut v) = ALLIES.lock() {
            v.clear();
        }
        if let Ok(mut v) = AMMO_ROLLED.lock() {
            v.clear();
        }
        if let Ok(mut v) = AMMO_FUNDING.lock() {
            v.clear();
        }
        USED_MASK.store(0, Ordering::Relaxed);
        LAST_DEPLOY.store(0, Ordering::Relaxed);
        BATTLE_LOGGED.store(false, Ordering::Relaxed);
        SUM_SPAWNED.store(0, Ordering::Relaxed);
        SUM_LOST.store(0, Ordering::Relaxed);
    }
    crumb!(
        "SPAWN template={template:#x} class={:#x} caller_rva={:#x} {} t={:.2}",
        if template != 0 { read::<u64>(template + OFF_CODE) } else { 0 },
        caller.wrapping_sub(IMAGE_BASE),
        scene(),
        read::<f64>(CLOCK)
    );
    diag::snapshot(format_args!("spawn caller_rva={:#x}", caller.wrapping_sub(IMAGE_BASE)));
    // spawns are rare and the point of the exercise: their own budget, never shared with ADD lines
    let level = level_name();
    let spawn_n = SPAWN_LINES.fetch_add(1, Ordering::Relaxed);
    let budget = line_budget();
    if spawn_n >= budget {
        if spawn_n == budget {
            log::info!("ALLY SPAWN lines: budget reached for this battle, the rest are not logged");
        }
        return;
    }
    log::info!(
        "ALLY SPAWN level={level} {} t={:.2} template={template:#x} class={:#x} caller_rva={:#x}",
        scene(),
        read::<f64>(CLOCK),
        if template != 0 { read::<u64>(template + OFF_CODE) } else { 0 },
        caller.wrapping_sub(IMAGE_BASE),
    );
}

/// Called at the start of `FUN_14006bf60` with its two arguments.
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

/// Roster entries (ship pointers).
unsafe fn roster() -> Vec<usize> {
    let (b, e): (usize, usize) = (read(ROSTER_BEGIN), read(ROSTER_END));
    if b < 0x10000 || e < b || (e - b) / LINK_STRIDE > SHIP_LIST_LIMIT {
        return Vec::new();
    }
    (0..(e - b) / LINK_STRIDE).map(|i| read::<usize>(b + i * LINK_STRIDE + 8)).filter(|p| *p != 0).collect()
}

type DeployFn = unsafe extern "C" fn(usize, usize, *mut u64, u64, f32, f32, f32, f32, u32, i32) -> u8;
type SetControlledFn = unsafe extern "C" fn(usize, usize, u32) -> usize;
type SetLinkFn = unsafe extern "C" fn(usize, usize);
type ExtractFn = unsafe extern "C" fn(usize, u32);
type GetProfileFn = unsafe extern "C" fn() -> usize;

/// Per-ship activity counters for the ally/enemy comparison.
#[derive(Default)]
struct Probe {
    who: usize,
    t0: f64,
    last_pos: (f32, f32),
    last_t: f64,
    last_zenith: f32,
    last_guns: f32,
    launches: u32,
    shots: u32,
}

static PROBE_ALLY: Mutex<Probe> = Mutex::new(Probe { who: 0, t0: 0.0, last_pos: (0.0, 0.0), last_t: 0.0, last_zenith: 0.0, last_guns: 0.0, launches: 0, shots: 0 });
static PROBE_ENEMY: Mutex<Probe> = Mutex::new(Probe { who: 0, t0: 0.0, last_pos: (0.0, 0.0), last_t: 0.0, last_zenith: 0.0, last_guns: 0.0, launches: 0, shots: 0 });

/// (entries, sum of loaded rounds, sum of reserve rounds, entries off cooldown) of a part list.
unsafe fn part_list(ship: usize, list: (usize, usize)) -> (usize, f32, i64, usize) {
    let (b, e): (usize, usize) = (read(ship + list.0), read(ship + list.1));
    if b < 0x10000 || e < b || (e - b) / LINK_STRIDE > SHIP_LIST_LIMIT {
        return (0, 0.0, 0, 0);
    }
    let (mut n, mut loaded, mut reserve, mut ready) = (0, 0.0f32, 0i64, 0);
    for a in (b..e).step_by(LINK_STRIDE) {
        let part: usize = read(a + 8);
        if part == 0 {
            continue;
        }
        n += 1;
        loaded += read::<f32>(part + OFF_PART_LOADED);
        reserve += read::<i32>(part + OFF_PART_RESERVE) as i64;
        if read::<f32>(part + OFF_PART_COOLDOWN) <= 0.0 {
            ready += 1;
        }
    }
    (n, loaded, reserve, ready)
}

impl Probe {
    fn reset(&mut self, who: usize, now: f64) {
        *self = Probe { who, t0: now, last_t: now, ..Default::default() };
        if who != 0 {
            unsafe {
                self.last_pos = (read(who + OFF_POS_X), read(who + OFF_POS_Y));
                self.last_zenith = part_list(who, OFF_ZENITH).1;
                self.last_guns = part_list(who, OFF_GUNS).1;
            }
        }
    }

    /// Called every frame: counts a launch whenever the launchers' loaded total drops, a shot
    /// whenever the weapons' loaded total drops (a reload raises it again, which is ignored).
    unsafe fn count(&mut self) {
        if self.who == 0 {
            return;
        }
        let zenith = part_list(self.who, OFF_ZENITH).1;
        let guns = part_list(self.who, OFF_GUNS).1;
        if zenith < self.last_zenith - 0.5 {
            self.launches += 1;
        }
        if guns < self.last_guns - 0.5 {
            self.shots += (self.last_guns - guns).round() as u32;
        }
        self.last_zenith = zenith;
        self.last_guns = guns;
    }

    /// One text line for the log.
    unsafe fn line(&mut self, tag: &str, now: f64, creatures: &ProbeCreatures) -> String {
        let ship = self.who;
        let pos = (read::<f32>(ship + OFF_POS_X), read::<f32>(ship + OFF_POS_Y));
        let dt = (now - self.last_t).max(0.001) as f32;
        let speed = ((pos.0 - self.last_pos.0).powi(2) + (pos.1 - self.last_pos.1).powi(2)).sqrt() / dt;
        self.last_pos = pos;
        self.last_t = now;
        let target: usize = read(ship + OFF_TARGET);
        // A target link alone does not establish a live creature. Non-creature or
        // stale targets have an unknown distance; never follow them for logging.
        let target_dist = if probe_ship_in_battle(target, creatures) {
            ((pos.0 - read::<f32>(target + OFF_POS_X)).powi(2) + (pos.1 - read::<f32>(target + OFF_POS_Y)).powi(2)).sqrt()
        } else {
            -1.0
        };
        let (zn, zl, zr, zready) = part_list(ship, OFF_ZENITH);
        let (gn, gl, gr, gready) = part_list(ship, OFF_GUNS);
        let flags: [u8; 5] = read(ship + OFF_FLAGS);
        format!(
            "ALLY probe {tag} t={:.0} c={ship:#x} pos=({:.0},{:.0}) speed={speed:.0} state={} mode={} target={target:#x} target_dist={target_dist:.0} aggr={:.2} retarget={:.1} \
             flags={flags:?} d60={:.1} launch_cnt={} | zenith n={zn} loaded={zl:.0} reserve={zr} ready={zready} launches={} | guns n={gn} loaded={gl:.0} reserve={gr} ready={gready} shots={}",
            now - self.t0,
            pos.0,
            pos.1,
            read::<i32>(ship + OFF_STATE),
            read::<i32>(ship + OFF_MODE_D70),
            read::<f32>(ship + OFF_AGGRESSION),
            read::<f32>(ship + OFF_RETARGET),
            read::<f32>(ship + OFF_LAUNCH_CD),
            read::<i32>(ship + OFF_LAUNCH_COUNT),
            self.launches,
            self.shots,
        )
    }
}

/// Current creature identities for one diagnostic callback. Never retained across ticks.
/// An unreadable or malformed list is not evidence that any cached object is alive.
struct ProbeCreatures {
    begin: usize,
    end: usize,
}

impl ProbeCreatures {
    /// # Safety
    /// The validated native span must remain alive and unchanged on the game thread. Only
    /// scalar pointer values are borrowed; callers must establish membership before following one.
    unsafe fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        (self.begin..self.end).step_by(LINK_STRIDE).map(|entry| {
            // SAFETY: construction checked every page of the bounded list, and the caller
            // guarantees its lifetime for this callback. This does not copy a native Link.
            unsafe { read::<usize>(entry + 8) }
        })
    }
}

/// Validate a bounded, callback-local view of the current native creature list.
///
/// # Safety
/// Call only from the battle hook on the game thread, while the native creature
/// list and its objects cannot be removed concurrently. No native ownership is taken.
unsafe fn probe_creatures() -> Option<ProbeCreatures> {
    let (Some(begin), Some(end)) = (diag::rd::<usize>(CREATURES_BEGIN), diag::rd::<usize>(CREATURES_END)) else {
        return None;
    };
    if begin < 0x10000 || end < begin {
        return None;
    }
    let list_bytes = end - begin;
    if !list_bytes.is_multiple_of(LINK_STRIDE) || list_bytes / LINK_STRIDE > CREATURE_LIST_LIMIT {
        return None;
    }
    // SAFETY: the header has been range/stride/cap checked above. This is the game-thread
    // battle callback, with no intervening native mutations during diagnostic collection.
    if !diag::readable_span(begin, list_bytes) {
        return None;
    }
    Some(ProbeCreatures { begin, end })
}

/// Validate membership before inspecting any cached pointer; readable memory is not identity.
///
/// # Safety
/// `creatures` must be this callback's validated native list view, on the game thread,
/// with no intervening native operation that can destroy or replace its objects.
unsafe fn probe_ship_in_battle(ship: usize, creatures: &ProbeCreatures) -> bool {
    if !plausible(ship) || !creatures.iter().any(|current| current == ship) {
        return false;
    }
    diag::rd::<u8>(ship + OFF_CODE).is_some_and(|class| class & 0x2f == 0x2f)
        && diag::rd::<i32>(ship + OFF_ACTIVE).is_some_and(|active| active != 0)
        && diag::rd::<i32>(ship + OFF_STATE).is_some_and(|state| !matches!(state, 6 | 7))
}

/// The enemy nearest to `from`: side below zero, in the level.
///
/// # Safety
/// `from` must be a current creature validated on the game thread by the caller.
unsafe fn nearest_enemy(from: usize, creatures: &ProbeCreatures) -> usize {
    let (fx, fy) = (read::<f32>(from + OFF_POS_X), read::<f32>(from + OFF_POS_Y));
    let mut best = (usize::MAX as f32, 0usize);
    for p in creatures.iter() {
        if !plausible(p)
            || !diag::rd::<u8>(p + OFF_CODE).is_some_and(|class| class & 0x2f == 0x2f)
            || !diag::rd::<i32>(p + OFF_ACTIVE).is_some_and(|active| active != 0)
            || !diag::rd::<i32>(p + OFF_STATE).is_some_and(|state| !matches!(state, 6 | 7))
            || !diag::rd::<f32>(p + OFF_SIDE).is_some_and(|side| side < 0.0)
        {
            continue;
        }
        let d = (read::<f32>(p + OFF_POS_X) - fx).powi(2) + (read::<f32>(p + OFF_POS_Y) - fy).powi(2);
        if d < best.0 {
            best = (d, p);
        }
    }
    best.1
}

/// While an ally is being tracked, one `ALLY track` line per game second for 60 s: where it is, where it
/// wants to go, what it targets, and where the player is.
unsafe fn track_ally(ships: &[usize]) {
    let ally = TRACK_ALLY.load(Ordering::Relaxed);
    if ally == 0 || !diag::debug() {
        return;
    }
    let now = read::<f64>(CLOCK);
    if !ships.contains(&ally) || now > f64::from_bits(TRACK_END.load(Ordering::Relaxed)) {
        log::info!("ALLY track ended (still in roster: {})", ships.contains(&ally));
        TRACK_ALLY.store(0, Ordering::Relaxed);
        return;
    }
    let Some(creatures) = probe_creatures() else {
        return;
    };
    // every frame: count launches and shots of the ally and of its nearest enemy
    if !probe_ship_in_battle(ally, &creatures) {
        return;
    }
    if let (Ok(mut a), Ok(mut e)) = (PROBE_ALLY.lock(), PROBE_ENEMY.lock()) {
        if a.who != ally {
            a.reset(ally, now);
            e.reset(0, now);
        }
        // Check membership first: never read an old enemy's fields to decide
        // whether that pointer still belongs to an enemy ship.
        if !probe_ship_in_battle(e.who, &creatures) || !diag::rd::<f32>(e.who + OFF_SIDE).is_some_and(|side| side < 0.0) {
            let enemy = nearest_enemy(ally, &creatures);
            e.reset(enemy, now);
        }
        a.count();
        e.count();
        if now >= f64::from_bits(TRACK_NEXT.load(Ordering::Relaxed)) {
            log::info!("{}", a.line("ally ", now, &creatures));
            if e.who != 0 {
                log::info!("{}", e.line("enemy", now, &creatures));
            }
        }
    }
    if now < f64::from_bits(TRACK_NEXT.load(Ordering::Relaxed)) {
        return;
    }
    TRACK_NEXT.store((now + 1.0).to_bits(), Ordering::Relaxed);
    let player = ships.iter().copied().find(|&p| read::<i32>(p + OFF_MODE_D70) == 1);
    let (px, py) = player.map_or((0.0, 0.0), |p| (read::<f32>(p + OFF_POS_X), read::<f32>(p + OFF_POS_Y)));
    let (ax, ay) = (read::<f32>(ally + OFF_POS_X), read::<f32>(ally + OFF_POS_Y));
    log::info!(
        "ALLY track t={now:.1} ally=({ax:.0},{ay:.0}) want=({:.0},{:.0}) player=({px:.0},{py:.0}) dist={:.0} state={} target={:#x} ai_flag={} mode={}",
        read::<f32>(ally + OFF_WAYPOINT_X),
        read::<f32>(ally + OFF_WAYPOINT_Y),
        ((ax - px).powi(2) + (ay - py).powi(2)).sqrt(),
        read::<i32>(ally + OFF_STATE),
        read::<usize>(ally + OFF_TARGET),
        read::<u8>(ally + OFF_AI_FLAG),
        read::<i32>(ally + OFF_MODE_D70),
    );
}

static USED_MASK: AtomicU32 = AtomicU32::new(0);
static MAX_ALLIES: AtomicU32 = AtomicU32::new(1);
static ALLIES: Mutex<Vec<usize>> = Mutex::new(Vec::new());
/// Whether `ship` is an ally this mod deployed and still tracks in the current battle.
fn tracked_ally(ship: usize) -> bool {
    ARMED.load(Ordering::Relaxed) && ALLIES.try_lock().is_ok_and(|allies| allies.contains(&ship))
}
static AMMO_ON: AtomicBool = AtomicBool::new(false);
/// Allies that have had their ammunition doctrine rolled this battle.
static AMMO_ROLLED: Mutex<Vec<usize>> = Mutex::new(Vec::new());
/// Per-gun ammo index and extra index that the battle reserve was funded with, recorded when the ally is
/// deployed (before the AI starts re-setting `+0x850`). `(creature, gun, index, extra_index)`.
static AMMO_FUNDING: Mutex<Vec<(usize, usize, i32, i32)>> = Mutex::new(Vec::new());
static LAST_DEPLOY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static BATTLE_LOGGED: AtomicBool = AtomicBool::new(false);
static SUM_SPAWNED: AtomicU32 = AtomicU32::new(0);
static SUM_LOST: AtomicU32 = AtomicU32::new(0);
// per-battle summary (logged when the roster empties)
static SUM_T0: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SUM_ENEMIES: AtomicU32 = AtomicU32::new(0);
static SUM_SQUAD: AtomicU32 = AtomicU32::new(0);
static SUM_ALLY_STATE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
static SUM_RESULT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
static SUM_FLAGS: AtomicU32 = AtomicU32::new(0);
static SUM_LAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
const F_SPAWNED: u32 = 1;
const F_HANDOVER: u32 = 2;
const F_EXTRACTED: u32 = 4;
const F_OFF: u32 = 8;
const F_FLAGSHIP_LEFT: u32 = 16;
const F_RETREAT_KEY: u32 = 32;
const BATTLE_RESULT: usize = 0x1_47fc_7004;
const ENEMIES_BEGIN: usize = 0x1_47fc_73d8;
const ENEMIES_END: usize = 0x1_47fc_73e0;
/// ammoSelectButtonPress? (0x14026f880): rebuilds the native ammo selector after a press.
const AMMO_PRESS: usize = 0x1_4026_f880;
const AMMO_PRESS_STOLEN: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10];
/// ReturnCreatureGunAmmoToInventory (0x1402698c0, RCX = creature): the end-of-battle refund. It credits each gun's whole
/// pool (`+0x858` i32, `+0x85c` i32, `+0x860` f32, `+0x868` i32) to the gun's CURRENT ammo index in the profile when that
/// ammo has a price. Every gun starts the battle with a free 9999-round pool of its base shell (InitBattleGunAmmoReserves
/// 0x14025fa30), so an ally switched to AP / proximity by `ally_ammo` would mint 9999 priced rounds per gun.
const REFUND: usize = 0x1_4026_98c0;
const REFUND_STOLEN: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10];
const REFUND_GUN_LISTS: [usize; 2] = [
    offset_of!(Creature, gun_group_0),
    offset_of!(Creature, gun_group_1),
];

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Remembers the ammo indices each gun of a freshly deployed ally was funded with. The AI controller re-sets
/// `+0x850` every tick from the doctrine flags, so by the end of the battle the index no longer says what the
/// reserve was bought as (or that it was the free 9999 base pool).
unsafe fn record_ammo_funding(creature: usize) -> usize {
    let mut n = 0;
    let Ok(mut funding) = AMMO_FUNDING.lock() else { return 0 };
    for list in REFUND_GUN_LISTS {
        let (b, e): (usize, usize) = (read(creature + list), read(creature + list + 8));
        if b < 0x10000 || e < b || (e - b) / LINK_STRIDE > SHIP_LIST_LIMIT {
            continue;
        }
        for a in (b..e).step_by(LINK_STRIDE) {
            let gun: usize = read(a + 8);
            if !plausible(gun) {
                continue;
            }
            let entry = (
                creature,
                gun,
                read::<i32>(gun + offset_of!(Body, weapon_ammo)),
                read::<i32>(gun + offset_of!(Body, weapon_ammo_extra)),
            );
            if let Some(slot) = funding.iter_mut().find(|r| r.1 == gun) {
                *slot = entry;
            } else {
                funding.push(entry);
            }
            n += 1;
        }
    }
    n
}

/// Entry hook of the refund: for a creature whose ammo doctrine we rolled, put every gun back on the ammo
/// index its reserve was funded with, so the original credits paid rounds under the right type and credits
/// nothing for the free 9999 base pool.
///
/// It must not touch the pre-combat screen: the same fleet ship keeps its pointer across battles and is only
/// dropped from `AMMO_ROLLED` at the next battle start, while the game refunds a ship's guns every time the
/// player picks an ammo type there. Zeroing then threw away the share the player had just chosen (ammo
/// vanished from the inventory when the same type was picked for the player and the ally).
unsafe extern "C" fn on_refund(creature: usize) {
    let _hook = diag::scope("on_refund");
    let ours = AMMO_ROLLED.lock().map_or(false, |v| v.contains(&creature));
    if !ours {
        return;
    }
    // while the pre-combat screen is up the game refunds on every ammo pick; a partial list is treated as
    // "screen still up" so an uncertain scan cannot wipe the share the player just chose
    if precombat::has_cards_or_uncertain() {
        return;
    }
    let Ok(funding) = AMMO_FUNDING.lock() else { return };
    let (mut restored, mut unrecorded) = (0, 0);
    for list in REFUND_GUN_LISTS {
        let (b, e): (usize, usize) = (read(creature + list), read(creature + list + 8));
        if b < 0x10000 || e < b || (e - b) / LINK_STRIDE > SHIP_LIST_LIMIT {
            continue;
        }
        for a in (b..e).step_by(LINK_STRIDE) {
            let gun: usize = read(a + 8);
            if !plausible(gun) {
                continue;
            }
            if let Some(&(_, _, index, extra)) = funding.iter().find(|r| r.0 == creature && r.1 == gun) {
                (gun as *mut i32).byte_add(offset_of!(Body, weapon_ammo)).write_unaligned(index);
                (gun as *mut i32).byte_add(offset_of!(Body, weapon_ammo_extra)).write_unaligned(extra);
                restored += 1;
            } else {
                // no funding index (should not happen for an ally we deployed): keep the old safeguard
                for off in [
                    offset_of!(Body, weapon_ammo_amount),
                    offset_of!(Body, weapon_ammo_extra_amount),
                    offset_of!(Body, weapon_reload_step_rounds),
                ] {
                    ((gun + off) as *mut i32).write_unaligned(0);
                }
                ((gun + offset_of!(Body, weapon_ammo_loaded)) as *mut f32).write_unaligned(0.0);
                unrecorded += 1;
            }
        }
    }
    log::info!(
        "ALLY refund: c={creature:#x} {restored} guns restored to their funded ammo index, {unrecorded} without a record emptied"
    );
}

unsafe extern "C" fn on_ammo_press() {
    let _hook = diag::scope("on_ammo_press");
    precombat::inspect_gui("ammoSelectButtonPress");
}

/// Moves reserve entry `k` (0 = the front) to the front so the vanilla deploy picks that ship.
unsafe fn reserve_to_front(k: usize) -> bool {
    let sentinel: usize = read(RESERVE);
    if sentinel == 0 {
        return false;
    }
    let mut node: usize = read(sentinel);
    for _ in 0..k {
        if node == 0 || node == sentinel {
            return false;
        }
        node = read(node);
    }
    if node == 0 || node == sentinel {
        return false;
    }
    if k == 0 {
        return true;
    }
    let (next, prev): (usize, usize) = (read(node), read(node + 8));
    (prev as *mut usize).write_unaligned(next); // prev.next = next
    ((next + 8) as *mut usize).write_unaligned(prev); // next.prev = prev
    let first: usize = read(sentinel);
    (node as *mut usize).write_unaligned(first);
    ((node + 8) as *mut usize).write_unaligned(sentinel);
    ((first + 8) as *mut usize).write_unaligned(node);
    (sentinel as *mut usize).write_unaligned(node);
    true
}


/// A pointer that can be a live game object: user-space, 8-aligned, not a small integer.
fn plausible(p: usize) -> bool {
    p >= 0x10000 && p < 0x7fff_ffff_0000 && p % 8 == 0
}

/// A creature that is still part of the battle: plausible pointer, creature class code, active, not dead.
unsafe fn live_ship(p: usize) -> bool {
    plausible(p) && read::<u8>(p + native_layout::NODE_CODE) & 0x2f == 0x2f && read::<i32>(p + OFF_ACTIVE) != 0 && !matches!(read::<i32>(p + OFF_STATE), 6 | 7)
}

/// Ships the F8 order has put on the bombing role (`+0xd70 = 2`): (ship, order before, checks that did not find it).
/// The role stays on the ship after it returns to the fleet (the game never resets it), so it is put back.
static ORDERED: Mutex<Vec<(usize, i32, u32)>> = Mutex::new(Vec::new());
const ORDER_BOMBING: i32 = 2;

/// Puts the order back on every ordered ship that is out of the battle and still in a player fleet. A ship that
/// is still fighting waits; a ship that is in no fleet (destroyed, object gone) is never touched.
/// `final_try`: the next battle starts, the list is cleared afterwards.
unsafe fn restore_orders(why: &str, final_try: bool) {
    let Ok(mut list) = ORDERED.lock() else { return };
    if list.is_empty() {
        return;
    }
    let in_battle = roster();
    list.retain_mut(|(ship, previous, misses)| {
        if in_battle.contains(ship) {
            return !final_try; // still fighting
        }
        if diag::fleet_has_ship(*ship) {
            let now = read::<i32>(*ship + OFF_MODE_D70);
            if now == ORDER_BOMBING {
                (*ship as *mut i32).byte_add(OFF_MODE_D70).write_unaligned(*previous);
                log::info!("ALLY order restored ({why}): c={:#x} order {now} -> {previous}", *ship);
            } else {
                log::info!("ALLY order not restored ({why}): c={:#x} order is {now} now, left alone", *ship);
            }
            return false;
        }
        *misses += 1;
        if final_try || *misses >= 40 {
            log::info!("ALLY order not restored ({why}): c={:#x} is in no player fleet (destroyed?), left alone", *ship);
            return false;
        }
        true
    });
}

/// Order the ally out: the AI-run, strike-craft "fly out" state (mode 2, state 2). The AI tick then steers to
/// the arena edge and the manager extracts the ship (it returns to the fleet if alive).
unsafe fn order_retreat(ally: usize, why: &str) -> bool {
    if !live_ship(ally) {
        log::info!("ALLY retreat ({why}): c={ally:#x} is not a live ship, ignored");
        return false;
    }
    let state = read::<i32>(ally + OFF_STATE);
    if matches!(state, 6 | 7) {
        log::info!("ALLY retreat ({why}): ally is dead (state {state}), nothing to do");
        return false;
    }
    if why == "key" {
        SUM_FLAGS.fetch_or(F_RETREAT_KEY, Ordering::Relaxed);
    }
    (ally as *mut u8).byte_add(OFF_AI_FLAG).write(1); // AI-run again (a promoted ally is player-flown)
    if let Ok(mut ordered) = ORDERED.lock() {
        if !ordered.iter().any(|e| e.0 == ally) {
            ordered.push((ally, read::<i32>(ally + OFF_MODE_D70), 0));
        }
    }
    (ally as *mut i32).byte_add(OFF_MODE_D70).write_unaligned(ORDER_BOMBING);
    (ally as *mut i32).byte_add(OFF_STATE).write_unaligned(2);
    log::info!(
        "ALLY retreat ordered ({why}): c={ally:#x} was state {state}, now mode_d70={} state={} ai_flag={}",
        read::<i32>(ally + OFF_MODE_D70),
        read::<i32>(ally + OFF_STATE),
        read::<u8>(ally + OFF_AI_FLAG),
    );
    flush_log();
    true
}

static FUEL_RETREAT_PERCENT: AtomicU32 = AtomicU32::new(0); // f32 bits
static FUEL_LOG_AT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0); // f64 bits of the game clock
const OFF_FUEL: usize = offset_of!(Creature, fuel); // f64: combat fuel now (the battle burns it: 0x14007a210)
const OFF_FUEL_CAPACITY: usize = offset_of!(Creature, fuel_capacity); // f64
const OFF_FUEL_AT_START: usize = 0xbd0; // f32: written at spawn (0x1402672c0), not modeled by highfleet-rs

/// Orders out every ally whose combat fuel is at or below the configured percent. Once per ship: a ship already
/// ordered out (F8 or fuel) stays in `ORDERED` until it is back in the fleet.
unsafe fn fuel_retreat(allies: &[usize]) {
    let percent = f32::from_bits(FUEL_RETREAT_PERCENT.load(Ordering::Relaxed));
    if percent <= 0.0 {
        return;
    }
    for &ally in allies {
        if !live_ship(ally) || read::<i32>(ally + OFF_STATE) == 2 {
            continue;
        }
        // a promoted ally is the ship the player flies
        if read::<i32>(ally + OFF_MODE_D70) == 1 && read::<u8>(ally + OFF_AI_FLAG) == 0 {
            continue;
        }
        if ORDERED.lock().map(|o| o.iter().any(|e| e.0 == ally)).unwrap_or(true) {
            continue;
        }
        let fuel: f64 = read(ally + OFF_FUEL);
        let basis: f64 = read(ally + OFF_FUEL_CAPACITY);
        if !fuel.is_finite() || !basis.is_finite() || basis <= 0.0 {
            continue;
        }
        let now = fuel / basis * 100.0;
        let clock = read::<f64>(CLOCK);
        if clock - f64::from_bits(FUEL_LOG_AT.load(Ordering::Relaxed)) >= 10.0 {
            FUEL_LOG_AT.store(clock.to_bits(), Ordering::Relaxed);
            log::info!(
                "ALLY fuel: c={ally:#x} combat fuel {fuel:.1}, capacity {:.1}, at battle start {:.1}, {now:.1}% of capacity",
                read::<f64>(ally + OFF_FUEL_CAPACITY),
                read::<f32>(ally + OFF_FUEL_AT_START)
            );
        }
        if now <= percent as f64 {
            log::info!(
                "ALLY fuel low: c={ally:#x} combat fuel {fuel:.1} of capacity {basis:.1} = {now:.1}% <= {percent}%, leaving"
            );
            order_retreat(ally, "fuel");
            play_radio(SND_DANGER, "fuel retreat");
        }
    }
}

/// Retreat key and automatic retreat; runs every battle frame.
unsafe fn retreat_logic(ships: &[usize]) {
    // an ally the manager promoted to the player's ship (mode 1, AI flag off) is the flagship now, not an ally
    if let Ok(mut v) = ALLIES.lock() {
        v.retain(|p| {
            let promoted = ships.contains(p) && read::<i32>(*p + OFF_MODE_D70) == 1 && read::<u8>(*p + OFF_AI_FLAG) == 0;
            if promoted {
                log::info!("ALLY c={:#x} was promoted to the flagship: no longer tracked as an ally", *p);
            }
            !promoted
        });
    }
    let allies: Vec<usize> = ALLIES.lock().map(|v| v.iter().copied().filter(|p| ships.contains(p)).collect()).unwrap_or_default();
    fuel_retreat(&allies);
    let key = RETREAT_KEY.load(Ordering::Relaxed);
    if key != 0 {
        let down = GetAsyncKeyState(key as i32) as u16 & 0x8000 != 0;
        if down && !KEY_WAS_DOWN.swap(true, Ordering::Relaxed) {
            // v0.9.2: only while the player's own ship is flying in the arena. During the fade-out after a retreat
            // (flagship leaving or gone) the scene is being torn down and the order would hit a dying ship.
            let flagship_flying = ships.iter().any(|&p| {
                !allies.contains(&p) && live_ship(p) && read::<i32>(p + OFF_MODE_D70) == 1 && read::<i32>(p + OFF_STATE) != 2
            });
            if !flagship_flying {
                log::info!("ALLY retreat key ignored: no flagship is flying (battle ending or handover pending)");
            }
            let mut ordered = false;
            for &ally in allies.iter().filter(|_| flagship_flying) {
                // never on a ship the player flies (e.g. an ally after the flagship left): it would turn it into an
                // AI strike run; that ship retreats by flying to the retreat point
                if read::<i32>(ally + OFF_MODE_D70) == 1 && read::<u8>(ally + OFF_AI_FLAG) == 0 {
                    log::info!("ALLY retreat key ignored: c={ally:#x} is the ship you fly");
                } else {
                    ordered |= order_retreat(ally, "key");
                }
            }
            if ordered {
                play_radio(SND_DANGER, "retreat key");
            }
        } else if !down {
            KEY_WAS_DOWN.store(false, Ordering::Relaxed);
        }
    }
    let point: u64 = read(RETREAT_POINT);
    if point != 0 {
        RETREAT_POINT_SAVED.store(point, Ordering::Relaxed);
    }
    // the flagship: a flown (mode 1) ship that is not an ally
    let flagship = ships.iter().copied().find(|&p| !allies.contains(&p) && read::<i32>(p + OFF_MODE_D70) == 1);
    if let Some(p) = flagship {
        FLAGSHIP.store(p, Ordering::Relaxed);
        FLAGSHIP_STATE.store(read::<i32>(p + OFF_STATE), Ordering::Relaxed);
        FLAGSHIP_ALIVE.store(read::<i32>(p + OFF_ACTIVE) != 0 && !matches!(read::<i32>(p + OFF_STATE), 6 | 7), Ordering::Relaxed);
        return;
    }
    let last = FLAGSHIP.load(Ordering::Relaxed);
    if last != 0 {
        FLAGSHIP.store(0, Ordering::Relaxed);
        let alive = FLAGSHIP_ALIVE.load(Ordering::Relaxed);
        // a retreat zeroes the retreat point; a shutdown or a loss leaves it as it was
        let used_point = read::<u64>(RETREAT_POINT) == 0 && RETREAT_POINT_SAVED.load(Ordering::Relaxed) != 0;
        log::info!(
            "ALLY flagship {last:#x} gone from the roster (last seen alive: {alive}, last state {}, retreat point used: {used_point})",
            FLAGSHIP_STATE.load(Ordering::Relaxed)
        );
        if alive {
            SUM_FLAGS.fetch_or(F_FLAGSHIP_LEFT, Ordering::Relaxed);
        }
        if alive && HANDOVER.load(Ordering::Relaxed) {
            SUM_FLAGS.fetch_or(F_HANDOVER, Ordering::Relaxed);
            let saved = RETREAT_POINT_SAVED.load(Ordering::Relaxed);
            let now: u64 = read(RETREAT_POINT);
            // only when somebody can still use it: an ally stays, or a reserve ship will take over
            let someone_left = !allies.is_empty() || read::<usize>(RESERVE + 8) > 0;
            if now == 0 && saved != 0 && someone_left {
                (RETREAT_POINT as *mut u64).write_unaligned(saved);
            }
            log::info!(
                "ALLY handover: flagship left, {} ally(s) stay and one is promoted; retreat point {:#x} -> {:#x} (saved {saved:#x})",
                allies.len(),
                now,
                read::<u64>(RETREAT_POINT),
            );
            flush_log();
        } else if alive {
            // The manager promotes roster[0] this very frame (no mode-1 ship left), so a fly-out order comes
            // too late: extract the allies now, the way the manager does for strike craft at the edge.
            let extract: ExtractFn = std::mem::transmute(EXTRACT);
            let before = ships.len();
            for &ally in &allies {
                extract(ally, 0);
            }
            SUM_FLAGS.fetch_or(F_EXTRACTED, Ordering::Relaxed);
            // no replacement ally after a retreat of the whole squad
            ALLY_DONE.store(true, Ordering::Relaxed);
            let after = roster();
            log::info!(
                "ALLY retreat with the flagship: extracted {} ally(s), roster {before} -> {}, reserve {}, wave timer {}",
                allies.len(),
                after.len(),
                read::<usize>(RESERVE + 8),
                read::<f32>(0x1_47fc_7020),
            );
        }
    }
}

/// Logs the `ALLY SUMMARY` line of the battle that just ended (once). `now` = the game clock of the end.
unsafe fn emit_summary(now: f64) {
    if SUM_T0.load(Ordering::Relaxed) == 0 {
        return;
    }
        let t0 = f64::from_bits(SUM_T0.swap(0, Ordering::Relaxed));
        let flags = SUM_FLAGS.load(Ordering::Relaxed);
        let mut result = read::<i32>(BATTLE_RESULT);
        if result == 0 {
            result = SUM_RESULT.load(Ordering::Relaxed);
        }
        let state = SUM_ALLY_STATE.load(Ordering::Relaxed);
        let (honor_player, honor_allies) = honor_share::totals();
        let ally = if flags & F_SPAWNED == 0 {
            if flags & F_OFF != 0 { "none (switched off)" } else { "none (not brought)" }.to_string()
        } else if flags & F_EXTRACTED != 0 {
            "retreated with the flagship".to_string()
        } else if matches!(state, 6 | 7) {
            format!("destroyed (state {state})")
        } else if flags & F_HANDOVER != 0 {
            format!("promoted after the flagship left (last state {state})")
        } else {
            format!("alive at the end (last state {state})")
        };
        log::info!(
            "ALLY SUMMARY v{}: {:.0} s, squad {} (flagship + {} others), enemies at start {}, result {result} ({}), flagship left alive: {}, retreat key used: {}, allies brought {} lost {}, ally: {ally}, honor {honor_player:.1} (allies {honor_allies:.1})",
            env!("CARGO_PKG_VERSION"),
            now - t0,
            SUM_SQUAD.load(Ordering::Relaxed),
            SUM_SQUAD.load(Ordering::Relaxed).saturating_sub(1),
            SUM_ENEMIES.load(Ordering::Relaxed),
            match result {
                -1 => "squad gone, enemies remain",
                1 => "enemies gone",
                2 => "both gone",
                _ => "unknown",
            },
            flags & F_FLAGSHIP_LEFT != 0,
            flags & F_RETREAT_KEY != 0,
            SUM_SPAWNED.load(Ordering::Relaxed),
            SUM_LOST.load(Ordering::Relaxed),
        );
}

/// Collects the per-battle numbers and logs one `ALLY SUMMARY` line when the battle is over.
unsafe fn summary_tick(ships: &[usize]) {
    let now = read::<f64>(CLOCK);
    if !ships.is_empty() {
        if SUM_T0.load(Ordering::Relaxed) == 0 {
            SUM_T0.store(now.max(0.001).to_bits(), Ordering::Relaxed);
            SUM_ENEMIES.store(count(ENEMIES_BEGIN, ENEMIES_END, 0x18) as u32, Ordering::Relaxed);
            SUM_SQUAD.store((ships.len() + read::<usize>(RESERVE + 8).min(32)) as u32, Ordering::Relaxed);
            SUM_ALLY_STATE.store(-1, Ordering::Relaxed);
            SUM_RESULT.store(0, Ordering::Relaxed);
            SUM_FLAGS.store(0, Ordering::Relaxed);
            honor_share::reset_totals();
        }
        SUM_LAST.store(now.to_bits(), Ordering::Relaxed);
        let result = read::<i32>(BATTLE_RESULT);
        if result != 0 {
            SUM_RESULT.store(result, Ordering::Relaxed);
        }
        let ally = ALLY_PTR.load(Ordering::Relaxed);
        if ally != 0 && ships.contains(&ally) {
            SUM_ALLY_STATE.store(read::<i32>(ally + OFF_STATE), Ordering::Relaxed);
        }
    } else {
        emit_summary(now);
    }
}

static SPARE_PRIZES: AtomicBool = AtomicBool::new(false);
static PRIZE_LINES: AtomicU32 = AtomicU32::new(0);
static LAST_PRIZE_KEY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Enemies (side < 0, in the level) that carry the prize flag.
unsafe fn prize_enemies() -> Vec<usize> {
    let (cb, ce): (usize, usize) = (read(CREATURES_BEGIN), read(CREATURES_END));
    (cb..ce)
        .step_by(0x18)
        .map(|a| read::<usize>(a + 8))
        .filter(|&p| p != 0 && read::<i32>(p + OFF_ACTIVE) != 0 && read::<f32>(p + OFF_SIDE) < 0.0 && read::<u8>(p + OFF_PRIZE) != 0)
        .collect()
}

/// The ship (creature) a target node belongs to: a creature (class code 47, `(code & 0x2f) == 0x2f` as the game's
/// own check in `FUN_140079810`) is its own ship; any other node (a part) points to its owning creature at +0xb8
/// (structures.md: "owner pointer"). The first version climbed the parent chain (+0x88) like the game's
/// `traverseLinkedListCreature_`, which gives the root PART of a chain, not the ship: it read the prize flag on a
/// wrong object and re-targeted every ally every tick (log of 2026-10-16, v0.9.0).
unsafe fn creature_of(node: usize) -> usize {
    if read::<u8>(node + native_layout::NODE_CODE) & 0x2f == 0x2f {
        return node;
    }
    let owner: usize = read(node + 0xb8);
    if owner != 0 && read::<u8>(owner + native_layout::NODE_CODE) & 0x2f == 0x2f {
        owner
    } else {
        node
    }
}

/// Runs right after the AI has chosen a target for `ship`. An AI-run ship of the player's side (an ally) that
/// picked a prize ship takes the nearest other enemy instead, or no target when only prizes are left.
unsafe extern "C" fn on_target_chosen(ship: usize) {
    let _hook = diag::scope("on_target_chosen");
    if !plausible(ship) || !SPARE_PRIZES.load(Ordering::Relaxed) {
        return;
    }
    // v0.9.3: only the ships this mod brought in (not the vanilla defenders of a missile or plane defense battle)
    if !ARMED.load(Ordering::Relaxed) {
        return;
    }
    match ALLIES.try_lock() {
        Ok(v) if v.contains(&ship) => {}
        _ => return,
    }
    // friendly and AI-run: the player's side, AI flag set (the ship the player flies has it cleared)
    if read::<f32>(ship + OFF_SIDE) <= 0.0 || read::<u8>(ship + OFF_AI_FLAG) == 0 {
        return;
    }
    let node: usize = read(ship + OFF_TARGET);
    if !plausible(node) {
        return;
    }
    // the target may be a part of an enemy ship: the prize flag is on the ship
    let target = creature_of(node);
    if !plausible(target) || read::<u8>(target + OFF_PRIZE) == 0 {
        return;
    }
    let (sx, sy) = (read::<f32>(ship + OFF_POS_X), read::<f32>(ship + OFF_POS_Y));
    let (cb, ce): (usize, usize) = (read(CREATURES_BEGIN), read(CREATURES_END));
    if cb < 0x10000 || ce < cb || (ce - cb) / LINK_STRIDE > CREATURE_LIST_LIMIT {
        return;
    }
    let mut best: Option<(f32, usize)> = None;
    for a in (cb..ce).step_by(LINK_STRIDE) {
        let p: usize = read(a + 8);
        if !plausible(p) || p == target || read::<i32>(p + OFF_ACTIVE) == 0 || read::<f32>(p + OFF_SIDE) >= 0.0 || read::<u8>(p + OFF_PRIZE) != 0 {
            continue;
        }
        if matches!(read::<i32>(p + OFF_STATE), 6 | 7) {
            continue;
        }
        let (dx, dy) = (read::<f32>(p + OFF_POS_X) - sx, read::<f32>(p + OFF_POS_Y) - sy);
        let d = dx * dx + dy * dy;
        if best.map_or(true, |(bd, _)| d < bd) {
            best = Some((d, p));
        }
    }
    let set_link: SetLinkFn = std::mem::transmute(SET_LINK);
    let replacement = best.map_or(0, |(_, p)| p);
    // the target may have been replaced or freed by now: only write while the ship still points at what we checked
    if !live_ship(ship) || read::<usize>(ship + OFF_TARGET) != node {
        return;
    }
    set_link(ship + OFF_TARGET_LINK, replacement);
    let key = (ship ^ target.rotate_left(17) ^ replacement.rotate_left(34)) as u64;
    if LAST_PRIZE_KEY.swap(key, Ordering::Relaxed) != key && PRIZE_LINES.fetch_add(1, Ordering::Relaxed) < 60 {
        log::info!("ALLY target: c={ship:#x} would attack the prize ship c={target:#x}; now {}", if replacement == 0 { "no target (only prizes left)".to_string() } else { format!("c={replacement:#x}") });
    }
}

/// Called at the start of the per-frame battle manager.
unsafe extern "C" fn on_battle_tick() {
    let _hook = diag::scope("on_battle_tick");
    plane_loadouts::verify(read::<f64>(CLOCK));
    plane_loadouts::tick();
    precombat::tick(1);
    let ships = roster();
    restore_orders("battle tick", false);
    ship_switch::tick(&ships);
    if !ARMED.load(Ordering::Relaxed) {
        // not an ally battle: touch nothing; just keep the roster-empty bookkeeping so a later battle starts clean
        if ships.is_empty() {
            PLAYER_SEEN.store(0, Ordering::Relaxed);
        }
        return;
    }
    track_ally(&ships);
    retreat_logic(&ships);
    // A manual handoff can introduce a new AI ally even after the reserve is exhausted.
    if AMMO_ON.load(Ordering::Relaxed) {
        if let (Ok(allies), Ok(mut rolled)) = (ALLIES.lock(), AMMO_ROLLED.lock()) {
            for &a in allies.iter().filter(|ship| ships.contains(ship)) {
                if live_ship(a) && !rolled.contains(&a) && ally_ammo::roll(a) {
                    rolled.push(a);
                }
            }
        }
    }
    ally_sound_events(&ships);
    summary_tick(&ships);
    if !ships.is_empty() {
        precombat::hide("battle running");
    }
    if ships.is_empty() {
        // between battles: arm again
        if ALLY_DONE.swap(false, Ordering::Relaxed) {
            log::info!("ALLY battle over, armed again");
            flush_log();
        }
        PLAYER_SEEN.store(0, Ordering::Relaxed);
        return;
    }
    if ALLY_DONE.load(Ordering::Relaxed) {
        // nothing more to bring; allies already out are tracked above
        return;
    }
    // the ship the player flies: mode 1, in the level, not dead (state 6/7)
    let flying = ships.iter().any(|&p| {
        read::<i32>(p + OFF_MODE_D70) == 1
            && read::<i32>(p + OFF_ACTIVE) != 0
            && !matches!(read::<i32>(p + OFF_STATE), 6 | 7)
    });
    if !flying {
        return;
    }
    let now = read::<f64>(CLOCK);
    let seen = f64::from_bits(PLAYER_SEEN.load(Ordering::Relaxed));
    if seen == 0.0 {
        PLAYER_SEEN.store(now.to_bits(), Ordering::Relaxed);
        return;
    }
    if now - seen < f64::from_bits(ALLY_DELAY.load(Ordering::Relaxed)) {
        return;
    }

    // allies still fighting; the lost ones free a place for the next ship switched on
    let alive = {
        let Ok(mut allies) = ALLIES.lock() else { return };
        allies.retain(|p| {
            let dead = matches!(read::<i32>(*p + OFF_STATE), 6 | 7) || read::<i32>(*p + OFF_ACTIVE) == 0;
            if ships.contains(p) && dead {
                SUM_LOST.fetch_add(1, Ordering::Relaxed);
                log::info!("ALLY lost: c={:#x} destroyed (state {})", *p, read::<i32>(*p + OFF_STATE));
                flush_log();
                return false;
            }
            ships.contains(p)
        });
        allies.len() as u32
    };
    if alive >= MAX_ALLIES.load(Ordering::Relaxed) {
        return;
    }
    // at most one deploy per 3 game seconds
    let last = f64::from_bits(LAST_DEPLOY.load(Ordering::Relaxed));
    if last != 0.0 && now - last < 3.0 {
        return;
    }
    let reserve: usize = read(RESERVE + 8);
    if !BATTLE_LOGGED.swap(true, Ordering::Relaxed) {
        let (cb, ce): (usize, usize) = (read(CREATURES_BEGIN), read(CREATURES_END));
        let enemy = (cb..ce).step_by(0x18).map(|a| read::<usize>(a + 8)).find(|&p| p != 0 && read::<f32>(p + OFF_SIDE) < 0.0);
        log::info!(
            "ALLY battle t={now:.1}: roster {}, reserve {reserve}, prize enemies at the first tick {}, switched on {:#x}, max allies {}, enemy mode_d70={:?}",
            ships.len(),
            prize_enemies().len(),
            precombat::selected_mask(),
            MAX_ALLIES.load(Ordering::Relaxed),
            enemy.map(|p| read::<i32>(p + OFF_MODE_D70))
        );
    }
    if reserve == 0 || reserve > 32 {
        ALLY_DONE.store(true, Ordering::Relaxed);
        if SUM_SPAWNED.load(Ordering::Relaxed) == 0 {
            log::info!("ALLY no reserve ship to bring (reserve size {reserve})");
        } else {
            log::info!("ALLY no reserve ship left to bring in");
        }
        return;
    }

    // the next ship switched on that has not been brought yet; its place in the reserve is its number minus 2,
    // minus the lower-numbered ships already deployed
    let mask = precombat::selected_mask();
    let used = USED_MASK.load(Ordering::Relaxed);
    let candidate = (2..(2 + reserve + used.count_ones() as usize)).find(|n| *n <= 9 && mask & (1 << n) != 0 && used & (1 << n) == 0);
    let Some(slot) = candidate else {
        ALLY_DONE.store(true, Ordering::Relaxed);
        if SUM_SPAWNED.load(Ordering::Relaxed) == 0 {
            SUM_FLAGS.fetch_or(F_OFF, Ordering::Relaxed);
            log::info!("ALLY off for this battle (no ship switched on; reserve {reserve}, mask {mask:#x})");
        }
        return;
    };
    let lower_used = (used & ((1u32 << slot) - 1)).count_ones() as usize;
    let index = slot - 2 - lower_used.min(slot - 2);
    USED_MASK.fetch_or(1 << slot, Ordering::Relaxed);
    LAST_DEPLOY.store(now.to_bits(), Ordering::Relaxed);
    if index >= reserve {
        log::info!("ALLY ship number {slot}: reserve entry {index} does not exist (reserve {reserve}), skipped");
        return;
    }

    let deploy: DeployFn = std::mem::transmute(DEPLOY);
    let set_controlled: SetControlledFn = std::mem::transmute(SET_CONTROLLED);
    let set_link: SetLinkFn = std::mem::transmute(SET_LINK);
    let get_profile: GetProfileFn = std::mem::transmute(GET_PROFILE);

    let moved = reserve_to_front(index);
    log::info!("ALLY chosen ship number {slot} (reserve entry {index}), moved to the front: {moved}");
    let before = ships.len();
    let mut pos: u64 = 0;
    let ok = deploy(
        RESERVE,
        ROSTER_BEGIN,
        &mut pos,
        1,
        read::<f32>(ARENA_X_LOW) + 100.0,
        read::<f32>(ARENA_X_HIGH) - 100.0,
        read::<f32>(ARENA_Y_LOW) + 100.0,
        read::<f32>(DEPLOY_K),
        read::<f32>(DEPLOY_AGGR).to_bits(),
        ALLY_SLOT as i32,
    );
    let after = roster();
    log::info!(
        "ALLY deploy returned {ok}, roster {before} -> {}, reserve {reserve} -> {}",
        after.len(),
        read::<usize>(RESERVE + 8)
    );
    flush_log();
    if after.len() != before + 1 {
        return; // nothing new to fix up
    }
    let Some(&ally) = after.last() else { return };

    // Record the ammo the reserve was funded with before the AI controller starts re-setting `+0x850`.
    let funded = record_ammo_funding(ally);
    log::info!("ALLY ammo funding: c={ally:#x} recorded the funded ammo index of {funded} guns");

    // hand it back to the stock AI: not controlled (AI flag on), no control slot, wingman mode
    set_controlled(get_profile(), 0, ALLY_SLOT as u32);
    set_link(CONTROLLED_LINKS + ALLY_SLOT * 0x18, 0);
    (ally as *mut i32).byte_add(OFF_MODE_D70).write_unaligned(ALLY_MODE);
    if let Ok(mut allies) = ALLIES.lock() {
        allies.push(ally);
    }
    play_radio(SND_JOIN[pick(SND_JOIN.len())], "ally joins");
    if SUM_SPAWNED.fetch_add(1, Ordering::Relaxed) == 0 {
        TRACK_ALLY.store(ally, Ordering::Relaxed);
        ALLY_PTR.store(ally, Ordering::Relaxed);
        TRACK_NEXT.store(0, Ordering::Relaxed);
        TRACK_END.store((read::<f64>(CLOCK) + 60.0).to_bits(), Ordering::Relaxed);
    }
    SUM_FLAGS.fetch_or(F_SPAWNED, Ordering::Relaxed);
    let (zn, _, _, _) = part_list(ally, OFF_ZENITH);
    let (gn, _, _, _) = part_list(ally, OFF_GUNS);
    log::info!("ALLY loadout (ship {slot}): zenith launchers {zn}, guns {gn}");
    log::info!(
        "ALLY ready c={ally:#x} side={} ai_flag={} mode_d70={} state={} aggr={}",
        read::<f32>(ally + OFF_SIDE),
        read::<u8>(ally + OFF_AI_FLAG),
        read::<i32>(ally + OFF_MODE_D70),
        read::<i32>(ally + OFF_STATE),
        read::<f32>(ally + OFF_AGGRESSION),
    );
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
    MAX_ALLIES.store(config.max_allies.max(1), Ordering::Relaxed);
    AMMO_ON.store(config.ally_ai_ammo, Ordering::Relaxed);
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
    FUEL_RETREAT_PERCENT.store(config.ally_fuel_retreat_percent.clamp(0.0, 100.0).to_bits(), Ordering::Relaxed);
    let handover = match config.flagship_leaves.as_str() {
        "handover" => true,
        "retreat" => false,
        other => {
            log::warn!("hf_ally: flagship_leaves {other:?} is not \"handover\" or \"retreat\"; using \"handover\"");
            true
        }
    };
    HANDOVER.store(handover, Ordering::Relaxed);
    RETREAT_KEY.store(config.retreat_key, Ordering::Relaxed);
    ALLY_DELAY.store(config.ally_delay_seconds.max(0.0).to_bits(), Ordering::Relaxed);

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

    let found = std::slice::from_raw_parts(PLAY_SFX as *const u8, PLAY_SFX_PROLOGUE.len());
    if found == PLAY_SFX_PROLOGUE {
        RADIO_SOUNDS.store(config.ally_sounds, Ordering::Relaxed);
        BUTTON_SOUNDS.store(true, Ordering::Relaxed);
    } else {
        log::warn!("hf_ally: playSFXSound_ bytes differ ({found:02x?}), ally and button sounds off");
    }

    let mut session = PatchSession::new();
    // At the function entry RSP is 8 mod 16 and [RSP] is the return address. Replay the two
    // rsp-relative stores, pad 8 bytes to align, then call with the return address (now at
    // RSP+0xC8 behind the helper's 7 pushes and 0x88 bytes) in RDX; RCX is still the template.
    let mut trampoline = Trampoline::new();
    trampoline.bytes(&SPAWN_PROLOGUE);
    trampoline.bytes(&[0x48, 0x83, 0xec, 0x08]); // sub rsp, 8
    trampoline.preserved_call(
        on_spawn as *const (),
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
        let found = std::slice::from_raw_parts(REFUND as *const u8, REFUND_STOLEN.len());
        if found == REFUND_STOLEN {
            let mut trampoline = Trampoline::new();
            trampoline.bytes(&[0x48, 0x83, 0xec, 0x08]);
            trampoline.preserved_call(on_refund as *const (), &[], &[], ReturnType::None);
            trampoline.bytes(&[0x48, 0x83, 0xc4, 0x08]);
            trampoline.bytes(&REFUND_STOLEN);
            trampoline.relative_jump(REFUND + REFUND_STOLEN.len());
            session
                .detour_trampoline(REFUND, REFUND_STOLEN.len(), trampoline)
                .map_err(|e| e.to_string())?;
        } else {
            log::warn!("hf_ally: refund function bytes differ ({found:02x?}), not hooked: allies on AP / proximity would credit free ammo");
            AMMO_ON.store(false, Ordering::Relaxed);
        }
    }
    SPARE_PRIZES.store(config.spare_prizes, Ordering::Relaxed);
    if config.spare_prizes {
        let target_ok = std::slice::from_raw_parts(TARGET_FN as *const u8, 3) == [0x48, 0x8b, 0xc4];
        let sites_ok = TARGET_SITES.iter().all(|(site, bytes)| std::slice::from_raw_parts(*site as *const u8, 5) == bytes);
        if target_ok && sites_ok {
            for (site, bytes) in TARGET_SITES {
                let mut trampoline = Trampoline::new();
                trampoline.bytes(&[0x51, 0x48, 0x83, 0xec, 0x28]); // push rcx; sub rsp, 0x28
                trampoline.absolute_call(TARGET_FN as *const ());
                trampoline.bytes(&[0x48, 0x83, 0xc4, 0x28, 0x59]); // add rsp, 0x28; pop rcx
                trampoline.preserved_call(on_target_chosen as *const (), &[], &[], ReturnType::None);
                trampoline.relative_jump(site + bytes.len());
                session.detour_trampoline(site, bytes.len(), trampoline).map_err(|e| e.to_string())?;
            }
        } else {
            log::warn!("hf_ally: the AI target choice or its call sites differ, prize ships are not spared");
            SPARE_PRIZES.store(false, Ordering::Relaxed);
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
        ("deploy", DEPLOY, &DEPLOY_PROLOGUE[..]),
        ("set_controlled", SET_CONTROLLED, &SET_CONTROLLED_PROLOGUE[..]),
        ("set_link", SET_LINK, &SET_LINK_PROLOGUE[..]),
        ("get_profile", GET_PROFILE, &GET_PROFILE_PROLOGUE[..]),
        ("extract", EXTRACT, &EXTRACT_PROLOGUE[..]),
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
            on_battle_tick as *const (),
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
