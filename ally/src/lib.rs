//! PROTOTYPE, standalone. A player-side ship that fights beside you, run by the stock AI
//! (Steam 1.163 only). Background and addresses: `hf_notes/docs/findings/battle-ally.md`.
//!
//! v0.2.0 adds the **ally** (config `spawn_ally`, default off): once per battle, when the player's
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

/// A line for the debug build's megalog (a no-op, and the arguments are not even formatted, otherwise).
macro_rules! crumb {
    ($($arg:tt)*) => {
        if diag::DEBUG {
            diag::crumb(&format!($($arg)*));
        }
    };
}

mod ally_ammo;
mod ally_hud;
mod color_probe;
mod crash_fixes;
mod diag;
mod intercept;
mod max_enemy;
mod plane_loadouts;
#[cfg(feature = "telemetry")]
mod telemetry;

use std::{
    ffi::{c_char, CStr},
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Mutex,
    },
};

use patchy::{PatchSession, ProcessModule, ReturnType, Trampoline};
use serde::{Deserialize, Serialize};
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
const OFF_CODE: usize = 0x28;

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
/// The pre-combat screen builder `FUN_1402d7620` ends with `call 0x140275850` (TarkhanGUIDispatchOperator) here.
const PRECOMBAT_SEAM: usize = 0x1_402d_88bb;
const PRECOMBAT_SEAM_BYTES: [u8; 5] = [0xe8, 0x90, 0xcf, 0xf9, 0xff];
/// `FUN_1402d73a0` (builds the squad screen with `secondaryPrecombatButtons`): the two dispatcher calls in it.
const PRECOMBAT_B1_SEAM: usize = 0x1_402d_752a;
const PRECOMBAT_B1_BYTES: [u8; 5] = [0xe8, 0x21, 0xe3, 0xf9, 0xff];
const PRECOMBAT_B2_SEAM: usize = 0x1_402d_75f2;
const PRECOMBAT_B2_BYTES: [u8; 5] = [0xe8, 0x59, 0xe2, 0xf9, 0xff];
const PRECOMBAT_SEAM_TARGET: usize = 0x1_4027_5850;
/// `GUI_ELEMENTS`: a std::list sentinel pointer; node +0 next, +0x10 the GUI object; name at object +0xa40.
const GUI_LIST: usize = 0x1_43a1_3ac8;
const GUI_COUNT: usize = 0x1_43a1_3ad0;
const GUI_NAME: usize = 0xa40;
// --- GUI element recipe (read from the PRECOMBAT_OK block of 0x1402d7620, see precombat-screen.md) ---
const GUI_NEW: usize = 0x1_4033_4094; // operator new(size)
const GUI_NEW_PROLOGUE: [u8; 9] = [0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9];
const GUI_CTOR: usize = 0x1_4012_a910; // GUI_2(mem, 0)
const GUI_CTOR_PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08];
const GUI_SETPOS: usize = 0x1_4005_20b0; // setXposYpos(obj, Vec2*)
const GUI_SETPOS_PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x10, 0x57, 0x48, 0x81, 0xec, 0xb0];
const GUI_SETBG: usize = 0x1_4001_be50; // SetBackground_(gui, idx, sprite, f32, u8, i32)
const GUI_SETBG_PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10];
const GUI_RESET: usize = 0x1_4015_cc30; // clears the +0xab8/+0xad8/+0xaf8/+0xb18 strings
const GUI_RESET_PROLOGUE: [u8; 9] = [0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9];
const GUI_FIND: usize = 0x1_401a_9170; // FindGuiElement(char*)
const GUI_FIND_PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10];
const GUI_ROOT: usize = 0x1_47fc_6f88; // pointer to the global node every GUI is added to
/// SomethingInputTask: per-frame input polling on the game thread (hf_voice hooks it too).
const INPUT_TASK: usize = 0x1_4018_dbf0;
const INPUT_TASK_PROLOGUE: [u8; 11] = [0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x48, 0x89, 0x70, 0x10];
const CLICK_ENTRY: usize = 0x1_4012_f120;
const CLICK_STOLEN: [u8; 26] = [
    0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d, 0xac, 0x24, 0xf8, 0xd3, 0xff,
    0xff, 0xb8, 0x08, 0x2d, 0x00, 0x00,
];
/// Constants the initializer of PRECOMBAT_OK is called with (vtable +0x2c0).
const INIT_F2: usize = 0x1_4807_5ce8; // f32 globals (set at run time)
const INIT_F3: usize = 0x1_4807_5cec;
const INIT_SCALE: usize = 0x1_4035_4698; // f32
const INIT_DIM_A: usize = 0x1_43a1_58fc; // i32
const INIT_DIM_B: usize = 0x1_43a1_3e5c; // i32
const INIT_STR: [usize; 5] = [0x1_4035_2962, 0x1_4035_3b70, 0x1_4035_3b60, 0x1_4035_3b50, 0x1_4035_3628];
const INIT_ONE: usize = 0x1_4035_24d8; // f32 1.0
const BG_MINUS_ONE: usize = 0x1_4035_24ec; // f32 -1.0
/// The AI target choice `FUN_140078b40(ship)` is called from the per-ship driver at two sites (`mov rcx,rbx; call`).
const TARGET_FN: usize = 0x1_4007_8b40;
const TARGET_SITES: [(usize, [u8; 5]); 2] = [
    (0x1_4007_3b93, [0xe8, 0xa8, 0x4f, 0x00, 0x00]),
    (0x1_4007_3c73, [0xe8, 0xc8, 0x4e, 0x00, 0x00]),
];
const OFF_PRIZE: usize = 0xc19; // u8: the ship may be captured after the battle (ship names Pollux, Kadesh Mk1, ...)
const OFF_TARGET_LINK: usize = 0xaf8; // the Link whose node is the target at +0xb00
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
const OFF_MODE_D70: usize = 0xd70; // i32: 1 = flown by the player, 2 = wingman
const OFF_ACTIVE: usize = 0x48; // i32: non-zero while the ship is in the level

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

const OFF_SIDE: usize = 0xad4; // f32: negative = AI, positive = player, 0 = neutral
const OFF_AI_FLAG: usize = 0xbd4; // u8: non-zero = joins the AI vector when added
const OFF_AGGRESSION: usize = 0xbc4; // f32: >= 0.5 makes the driver call FUN_140079b20(ship, 1)
const OFF_STATE: usize = 0xbdc; // i32: driver state (1/2 run the AI tick)
const OFF_MODE: usize = 0xb48; // i32
const OFF_ORDER: usize = 0xc98; // i32: index given by the Missile scenario
const OFF_TARGET: usize = 0xb00; // pointer: current target (link)

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Config {
    /// Master switch.
    enabled: bool,
    /// At most this many `ALLY` lines are written per level / per session for spawns.
    max_lines: u32,
    /// Bring the next ship of the fleet into the battle as an AI wingman.
    spawn_ally: bool,
    /// Game seconds the player's ship must have been in the battle before the ally arrives.
    ally_delay_seconds: f64,
    /// When the flagship leaves the arena alive, order the ally out too (otherwise the battle goes on with the
    /// ally promoted to your ship and the retreat is lost).
    auto_retreat: bool,
    /// What `auto_retreat` does when the flagship leaves alive: "retreat" = the ally is extracted with it and the
    /// battle ends as a retreat; "handover" = the ally is promoted to your ship (the game's own behaviour) and the
    /// retreat point the flagship used up is put back, so the new flagship can still retreat.
    flagship_leaves: String,
    /// Virtual-key code of the "ally, retreat" key (0x77 = F8); 0 disables the key.
    retreat_key: u32,
    /// Add the "ALLY" on/off button to the pre-combat screen (needs `spawn_ally`; click it to bring or skip the ally).
    ally_toggle: bool,
    /// Log the GUI elements of the pre-combat screen each time it is built (step 1 of the "ally yes/no" button).
    log_precombat: bool,
    /// Per-second ally/enemy probe and track lines (very chatty; for AI analysis, off for ordinary play).
    probe_lines: bool,
    /// How many AI allies may fight at the same time. Ships switched on beyond that wait in the reserve and
    /// step in, in ship-number order, when an ally is destroyed or has left.
    max_allies: u32,
    /// Allies never keep a prize ship (the ships the player may capture and sell after the battle: the game flags
    /// them at creature +0xc19) as a target: they pick another enemy, or hold fire when only prizes are left.
    spare_prizes: bool,
    /// SIDE PATCH (not in master). `m_oid`s the AI never treats as a strategic launch to intercept in the missile-vs-ship
    /// arcade scene, e.g. `["MDL_MISSILE_03"]` (the R-9 Sprint) for a Sprint modded to be strat-launchable (its category has bit 0x8000000, so
    /// the CIWS and anti-air launchers of both sides would shoot at it). Copy the names from `m_oid=` in OL.seria. Empty = vanilla.
    intercept_ignore_oids: Vec<String>,
    /// Stock-game crash fixes (aircraft target choice, fleet-merge null fleet). Default on.
    fix_plane_ai_crash: bool,
    plane_bombers: bool,
    /// Offer the K-13 air-to-air loadout for carrier planes. Default off (janky).
    plane_k13: bool,
    plane_attack_delay: f32,
    plane_aa_range: f32,
    plane_aa_delay: f32,
    plane_aa_angle: f32,
    plane_aa_speed: f32,
    fix_merge_crash: bool,
    /// Allies roll the AI's AP / proximity shell doctrine once, like enemy ships do at spawn. Default on.
    ally_ai_ammo: bool,
    /// Most enemy ships in one battle (vanilla 5, clamped to 1..=127).
    max_enemy_ships: u32,
    /// Enemy ships placed per wave (vanilla 3, 1..=40). Waves refill every tick after the first 30 s.
    enemy_group_size: u32,
    /// Skip the 30 s wait before the enemy waves start (with `enemy_group_size` > 3: all enemies at once).
    enemy_all_at_once: bool,
    /// Pre-combat button that picks the armament of a carrier's planes (charged to the inventory). Default on.
    plane_loadouts: bool,
    log_colors: bool,
    ally_damage_blue: bool,
    ally_damage_color: u32,
    /// Log the whole GUI element list (names, positions, sizes) on every click and when the native ammo selector
    /// is rebuilt: to learn how the game lays out its own selectors (ammo icons, role icons).
    gui_inspect: bool,
    /// Log every creature registry pass (very noisy: every creature, every frame).
    log_creatures: bool,
    /// Battle telemetry for AI research (only in builds made with the cargo feature `telemetry`; ignored otherwise).
    telemetry: bool,
    /// Radio sounds for ally events: the retreat key, an ally joining the fight, an ally destroyed.
    ally_sounds: bool,
    /// An ally leaves the battle (same order as the retreat key) when its COMBAT fuel (the ship's own tank, the
    /// one the battle burns; not the map fuel of the fleet) falls to this percent. 0 = off. Allies only.
    ally_fuel_retreat_percent: f32,
    /// Text drawn next to ship number 1 on the pre-combat screen (the flagship, always your own ship); "" = none.
    marker_text: String,
    /// Render seam that draws the label: 22, 23 or 24 (24 is the topmost; a lower one may sit under the sliding doors).
    marker_seam: u32,
    /// The label stays hidden this many milliseconds after the screen is entered (the doors slide in).
    marker_delay_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            max_lines: 600,
            spawn_ally: true,
            ally_delay_seconds: 1.5,
            auto_retreat: true,
            flagship_leaves: "handover".to_string(),
            retreat_key: 0x77,
            ally_toggle: true,
            probe_lines: true,
            gui_inspect: false,
            spare_prizes: true,
            intercept_ignore_oids: Vec::new(),
            fix_plane_ai_crash: true,
            plane_bombers: false,
            plane_k13: false,
            plane_attack_delay: 6.0,
            plane_aa_range: 6000.0,
            plane_aa_delay: 3.0,
            plane_aa_angle: 0.35,
            plane_aa_speed: 1.5,
            fix_merge_crash: true,
            ally_ai_ammo: true,
            max_enemy_ships: 5,
            enemy_group_size: 3,
            enemy_all_at_once: false,
            plane_loadouts: true,
            log_colors: false,
            ally_damage_blue: true,
            ally_damage_color: 0x00e4ff,
            max_allies: 2,
            log_precombat: true,
            log_creatures: false,
            telemetry: false,
            ally_sounds: true,
            ally_fuel_retreat_percent: 0.0,
            marker_text: "PLAYER".to_string(),
            marker_seam: 24,
            marker_delay_ms: 0,
        }
    }
}

static GAME_IS_STEAM_1_163: AtomicBool = AtomicBool::new(false);
static MAX_LINES: AtomicU32 = AtomicU32::new(600);
static LINES: AtomicU32 = AtomicU32::new(0);
static SPAWN_LINES: AtomicU32 = AtomicU32::new(0);
static SPAWN_ALLY: AtomicBool = AtomicBool::new(false);
/// The ship mode (`+0xd70`) an ally gets: 0 = the plain AI ship the enemies use. It was the config key `ally_mode`
/// until v0.12.1; 1 (flown by a player) makes an ally look like the flagship and 2 (strike run) is the retreat order,
/// so only 0 is valid for an ally (hf_notes docs/findings/battle-ally.md, "ally_mode removed").
const ALLY_MODE: i32 = 0;
static ALLY_DONE: AtomicBool = AtomicBool::new(false);
/// v0.9.2/0.9.3: the ally logic runs only in a battle that was entered through the pre-combat screen (the card
/// choices come from it). Re-engaged chases, interceptions and other battles play as vanilla.
static PRECOMBAT_SEEN: AtomicBool = AtomicBool::new(false);
static ARMED: AtomicBool = AtomicBool::new(false);
static HAVE_PRECOMBAT_HOOK: AtomicBool = AtomicBool::new(false);

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
const OFF_POS_X: usize = 0x1840; // f32 creature centre
const OFF_POS_Y: usize = 0x1844;
const OFF_WAYPOINT_X: usize = 0xbf0; // f32 steering waypoint written by the AI tick
const OFF_WAYPOINT_Y: usize = 0xbf4;
// --- probe fields (all [code]/guess, see the findings note) ---
const OFF_RETARGET: usize = 0xb28; // f32 seconds until the next target re-roll
const OFF_FLAGS: usize = 0xbbc; // 5 control bytes: +0xbbc fires guns group 0 (LMB), +0xbbd group 1 (RMB), +0xbbe bombs, +0xbc0 flares
const OFF_LAUNCH_CD: usize = 0xd60; // f32 ship launch cooldown
const OFF_LAUNCH_COUNT: usize = 0xc98; // i32 per-ship launch counter
const OFF_ZENITH: (usize, usize) = (0x13d8, 0x13e0); // anti-ship launcher list
const OFF_GUNS: (usize, usize) = (0x1360, 0x1368); // weapon list
const OFF_PART_COOLDOWN: usize = 0x848; // f32
const OFF_PART_RESERVE: usize = 0x858; // i32
const OFF_PART_LOADED: usize = 0x860; // f32
/// Game time (f64 bits) at which the player's ship was first seen in this battle; 0 = not yet.
static HANDOVER: AtomicBool = AtomicBool::new(false);
/// The retreat point (x, y as two f32 packed in a u64) the game keeps at 0x147fc7638; zeroed when a ship uses it.
const RETREAT_POINT: usize = 0x1_47fc_7638;
static RETREAT_POINT_SAVED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static AUTO_RETREAT: AtomicBool = AtomicBool::new(false);
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

// --- radio sounds (v0.10.0) ---
// `playSFXSound_` (0x1402e30e0) is what the stock battle radio (`PlayRadioSounds_`, 0x1402d5910) ends in; the call
// below repeats its last call exactly: kind 0xb (radio), the sound resource name from `Media/Snd/sound.res`, the
// global GUI node, 60.0, then the stack arguments 1.0, a status float it writes into, and zeros.
const PLAY_SFX: usize = 0x1_402e_30e0;
const PLAY_SFX_PROLOGUE: [u8; 15] = [0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x48, 0x89, 0x68, 0x10, 0x48, 0x89, 0x70, 0x18];
const SFX_KIND_RADIO: i32 = 0xb;
type PlaySfxFn = unsafe extern "C" fn(i32, *const u8, usize, f32, f32, *mut f32, u64, u8, i32, u8, u8, f32, u8) -> f32;
static SOUNDS: AtomicBool = AtomicBool::new(false);
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

/// Plays a radio sound by its `sound.res` name (NUL-terminated); `what` is for the log.
unsafe fn play_radio(name: &[u8], what: &str) {
    if !SOUNDS.load(Ordering::Relaxed) {
        return;
    }
    let node: usize = read(GUI_ROOT);
    if node == 0 {
        log::info!("ALLY sound ({what}) skipped: no global node yet");
        return;
    }
    let slot = &SFX_STATUS[SFX_NEXT.fetch_add(1, Ordering::Relaxed) as usize % SFX_STATUS.len()];
    slot.store(0x3f80_0000, Ordering::Relaxed);
    let play: PlaySfxFn = std::mem::transmute(PLAY_SFX);
    let result = play(SFX_KIND_RADIO, name.as_ptr(), node, 60.0, 1.0, slot.as_ptr() as *mut f32, 0, 0, 0, 0, 0, 0.0, 1);
    log::info!("ALLY sound ({what}) {} -> {result}", String::from_utf8_lossy(&name[..name.len() - 1]));
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
    if LINES.fetch_add(1, Ordering::Relaxed) >= MAX_LINES.load(Ordering::Relaxed) {
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
        SPAWN_LINES.store(0, Ordering::Relaxed);
        PRIZE_LINES.store(0, Ordering::Relaxed);
        let armed = PRECOMBAT_SEEN.swap(false, Ordering::Relaxed) || !HAVE_PRECOMBAT_HOOK.load(Ordering::Relaxed);
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
    diag::snapshot(&format!("spawn caller_rva={:#x}", caller.wrapping_sub(IMAGE_BASE)));
    // spawns are rare and the point of the exercise: their own budget, never shared with ADD lines
    let level = level_name();
    let spawn_n = SPAWN_LINES.fetch_add(1, Ordering::Relaxed);
    if spawn_n >= MAX_LINES.load(Ordering::Relaxed) {
        if spawn_n == MAX_LINES.load(Ordering::Relaxed) {
            log::info!("ALLY SPAWN lines: max_lines reached for this battle, the rest are not logged");
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
    if b == 0 || e < b || (e - b) / 0x18 > 64 {
        return Vec::new();
    }
    (0..(e - b) / 0x18).map(|i| read::<usize>(b + i * 0x18 + 8)).filter(|p| *p != 0).collect()
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
    if b == 0 || e < b || (e - b) / 0x18 > 64 {
        return (0, 0.0, 0, 0);
    }
    let (mut n, mut loaded, mut reserve, mut ready) = (0, 0.0f32, 0i64, 0);
    for a in (b..e).step_by(0x18) {
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
    unsafe fn line(&mut self, tag: &str, now: f64) -> String {
        let ship = self.who;
        let pos = (read::<f32>(ship + OFF_POS_X), read::<f32>(ship + OFF_POS_Y));
        let dt = (now - self.last_t).max(0.001) as f32;
        let speed = ((pos.0 - self.last_pos.0).powi(2) + (pos.1 - self.last_pos.1).powi(2)).sqrt() / dt;
        self.last_pos = pos;
        self.last_t = now;
        let target: usize = read(ship + OFF_TARGET);
        let target_dist = if target != 0 {
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

/// The enemy nearest to `from`: side below zero, in the level.
unsafe fn nearest_enemy(from: usize) -> usize {
    let (cb, ce): (usize, usize) = (read(CREATURES_BEGIN), read(CREATURES_END));
    let (fx, fy) = (read::<f32>(from + OFF_POS_X), read::<f32>(from + OFF_POS_Y));
    let mut best = (usize::MAX as f32, 0usize);
    for a in (cb..ce).step_by(0x18) {
        let p: usize = read(a + 8);
        if p == 0 || read::<i32>(p + OFF_ACTIVE) == 0 || read::<f32>(p + OFF_SIDE) >= 0.0 {
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
    if ally == 0 || !PROBE_LINES.load(Ordering::Relaxed) {
        return;
    }
    let now = read::<f64>(CLOCK);
    if !ships.contains(&ally) || now > f64::from_bits(TRACK_END.load(Ordering::Relaxed)) {
        log::info!("ALLY track ended (still in roster: {})", ships.contains(&ally));
        TRACK_ALLY.store(0, Ordering::Relaxed);
        return;
    }
    // every frame: count launches and shots of the ally and of its nearest enemy
    if let (Ok(mut a), Ok(mut e)) = (PROBE_ALLY.lock(), PROBE_ENEMY.lock()) {
        if a.who != ally {
            a.reset(ally, now);
        }
        if e.who == 0 || read::<i32>(e.who + OFF_ACTIVE) == 0 {
            let enemy = nearest_enemy(ally);
            e.reset(enemy, now);
        }
        a.count();
        e.count();
        if now >= f64::from_bits(TRACK_NEXT.load(Ordering::Relaxed)) {
            log::info!("{}", a.line("ally ", now));
            if e.who != 0 {
                log::info!("{}", e.line("enemy", now));
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

static TOGGLE_ENABLED: AtomicBool = AtomicBool::new(false);
static LOG_PRECOMBAT: AtomicBool = AtomicBool::new(false);
/// Per-ship ally choice: bit n = ship number n (2..=9, as numbered on the pre-combat screen) may be the ally.
static SLOT_MASK: AtomicU32 = AtomicU32::new(1 << 2);
static USED_MASK: AtomicU32 = AtomicU32::new(0);
static MAX_ALLIES: AtomicU32 = AtomicU32::new(1);
static ALLIES: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static AMMO_ON: AtomicBool = AtomicBool::new(false);
/// Allies that have had their ammunition doctrine rolled this battle.
static AMMO_ROLLED: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static LAST_DEPLOY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static BATTLE_LOGGED: AtomicBool = AtomicBool::new(false);
static SUM_SPAWNED: AtomicU32 = AtomicU32::new(0);
static SUM_LOST: AtomicU32 = AtomicU32::new(0);
static TOGGLE_LINES: AtomicU32 = AtomicU32::new(0);
const TOGGLE_NAMES: [&[u8]; 8] = [
    b"ALLY_TOGGLE_2\0",
    b"ALLY_TOGGLE_3\0",
    b"ALLY_TOGGLE_4\0",
    b"ALLY_TOGGLE_5\0",
    b"ALLY_TOGGLE_6\0",
    b"ALLY_TOGGLE_7\0",
    b"ALLY_TOGGLE_8\0",
    b"ALLY_TOGGLE_9\0",
];
// the beta ORDER icons still exist in Midground2.res: "support" = a wingman, "unset" = a plain ship
const SPRITE_ON: &[u8] = b"order_support_on\0";
const SPRITE_OFF: &[u8] = b"order_unset\0";
/// Offset of a button from its ship card's position (GUI units, y down): above the card, under the number.
const TOGGLE_DY: f32 = -122.0;
/// Used when the cards report no usable positions: first card x, step, y.
const GRID_X0: f32 = -790.0;
const GRID_STEP: f32 = 190.0;
const GRID_Y: f32 = -275.0;
const TOGGLE_SIZE: f32 = 64.0;
/// The native ammo selector sits at the card's x about 100 above the card centre; ours goes right of the ship number.
const TOGGLE_DX: f32 = 70.0;
/// Width of the ship-number glyph's clear zone; the button must fit between two neighbouring numbers.
const NUMBER_WIDTH: f32 = 44.0;
const TOGGLE_MIN_SIZE: f32 = 40.0;
/// The cards must stand still this long (ms) before the buttons are placed again.
const SETTLE_MS: u64 = 150;
/// The slot-1 label ("PLAYER", text from a render seam): where (GUI units, centre origin, y down) and whether to draw.
static MARKER_X: AtomicU32 = AtomicU32::new(0);
static MARKER_SHOW: AtomicBool = AtomicBool::new(false);
static MARKER_TEXT: Mutex<Vec<u8>> = Mutex::new(Vec::new());
static MARKER_DRAWS: AtomicU32 = AtomicU32::new(0);
/// The label sits a little right of where a button would (it is wider than one).
const MARKER_DX: f32 = 14.0;
/// Render seam B24 (the call to the layer flush at the end of the battle-branch renderer; the same code draws the
/// pre-combat screen): checked as `E8 rel32` to FLUSH before use. Text is the elevation readout's way: fixed_16.
const MARKER_SEAMS: [usize; 3] = [0x1_4030_9327, 0x1_4030_9408, 0x1_4030_9897]; // B22, B23, B24
/// Which of them draws (config `marker_seam`, 22 to 24): a lower seam may be drawn under the sliding doors.
static MARKER_SEAM_SEL: AtomicU32 = AtomicU32::new(24);
/// The label stays hidden this long (ms) after the screen is entered, while the doors slide.
static MARKER_DELAY_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static ENTRY_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
const FLUSH: usize = 0x1_4008_3b50;
const MEDIA_RES: usize = 0x1_43a1_19e8;
const GET_FONT: usize = 0x1_4017_03e0;
const STYLE_FONT: usize = 0x1_401a_8710;
const FONT_RENDER: usize = 0x1_4016_eb40;
const MARKER_FONT: &std::ffi::CStr = c"flash_large_shadow";
type GetFontFn = unsafe extern "win64" fn(*mut u8, *const std::ffi::c_char) -> *mut u8;
type StyleFn = unsafe extern "win64" fn(*const std::ffi::c_char, f32, i32, u32) -> *mut u8;
type RenderTextFn = unsafe extern "win64" fn(*mut u8, f32, f32, u32, *const std::ffi::c_char, *mut f32, *mut f32, i32, f32) -> f32;
/// The buttons exist and are shown (set when they are placed, cleared when they are hidden).
static PRECOMBAT_LIVE: AtomicBool = AtomicBool::new(false);
static PLACED_SIG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static TICK_SIG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static TICK_CHANGED_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static TICK_LOGS: AtomicU32 = AtomicU32::new(0);
static TICKS_SEEN: [AtomicU32; 2] = [AtomicU32::new(0), AtomicU32::new(0)];
/// The plane-armament button is only drawn when its hook is installed.
static PLANE_BUTTONS: AtomicBool = AtomicBool::new(false);
/// Text beside each plane button, drawn from the render seam: (slot, x, y in GUI units, size, text with a NUL).
static PLANE_LABELS: Mutex<Vec<(usize, f32, f32, f32, Vec<u8>)>> = Mutex::new(Vec::new());
/// One button per card and plane type: index = craft * 9 + (card slot - 1); the click handler splits the number again.
const LOADOUT_NAMES: [&[u8]; 18] = [
    b"PLANE_ARM_1\0",
    b"PLANE_ARM_2\0",
    b"PLANE_ARM_3\0",
    b"PLANE_ARM_4\0",
    b"PLANE_ARM_5\0",
    b"PLANE_ARM_6\0",
    b"PLANE_ARM_7\0",
    b"PLANE_ARM_8\0",
    b"PLANE_ARM_9\0",
    b"PLANE_ARM_10\0",
    b"PLANE_ARM_11\0",
    b"PLANE_ARM_12\0",
    b"PLANE_ARM_13\0",
    b"PLANE_ARM_14\0",
    b"PLANE_ARM_15\0",
    b"PLANE_ARM_16\0",
    b"PLANE_ARM_17\0",
    b"PLANE_ARM_18\0",
];
static LOADOUT_GUIS: [std::sync::atomic::AtomicUsize; 18] = [const { std::sync::atomic::AtomicUsize::new(0) }; 18];
/// Gap (GUI units) between the ALLY button and the plane button above it.
const LOADOUT_GAP: f32 = 3.0;
/// The plane buttons are smaller than the ALLY button so both rows stay in the part of the screen that takes clicks.
const LOADOUT_SIZE: f32 = 44.0;
const LABEL_FONT: &std::ffi::CStr = c"flash_large_shadow";
static TOGGLE_GUIS: [std::sync::atomic::AtomicUsize; 8] = [const { std::sync::atomic::AtomicUsize::new(0) }; 8];
static LAST_TOGGLE_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static PROBE_LINES: AtomicBool = AtomicBool::new(false);
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
static GUI_INSPECT: AtomicBool = AtomicBool::new(false);
static INSPECT_DUMPS: AtomicU32 = AtomicU32::new(0);
static INSPECT_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// ammoSelectButtonPress? (0x14026f880): rebuilds the native ammo selector after a press.
const AMMO_PRESS: usize = 0x1_4026_f880;
const AMMO_PRESS_STOLEN: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10];
static PRECOMBAT_BUILDS: AtomicU32 = AtomicU32::new(0);

/// A NUL-terminated ASCII name at `address`, at most 48 bytes, printable only.
unsafe fn cstr(address: usize) -> String {
    (0..48)
        .map(|i| read::<u8>(address + i))
        .take_while(|&b| (0x20..0x7f).contains(&b))
        .map(|b| b as char)
        .collect()
}

/// Runs right before the pre-combat screen is handed to the GUI dispatcher: every control exists now.
unsafe extern "C" fn on_precombat() {
    let _hook = diag::scope("on_precombat A");
    precombat_dump("A 0x1402d7620 end");
}
unsafe extern "C" fn on_precombat_b1() {
    let _hook = diag::scope("on_precombat B1");
    precombat_dump("B1 0x1402d73a0 after buttons");
}
unsafe extern "C" fn on_precombat_b2() {
    let _hook = diag::scope("on_precombat B2");
    PRECOMBAT_SEEN.store(true, Ordering::Relaxed);
    precombat_dump("B2 0x1402d73a0 end");
}

type NewFn = unsafe extern "C" fn(usize) -> *mut u8;
type CtorFn = unsafe extern "C" fn(*mut u8, i32) -> *mut u8;
type GuiInitFn = unsafe extern "C" fn(*mut u8, *const u8, f32, f32, f32, f32, *const u8, *const u8, *const u8, *const u8, *const u8, f32);
type NodeAddFn = unsafe extern "C" fn(*mut u8, *mut u8, usize, usize);
type SetBgFn = unsafe extern "C" fn(*mut u8, i32, *const u8, f32, u8, i32) -> *mut u8;
type ResetFn = unsafe extern "C" fn(*mut u8);
type SetPosFn = unsafe extern "C" fn(*mut u8, *const f32);
type FindFn = unsafe extern "C" fn(*const u8) -> usize;

fn toggle_line(message: String) {
    if TOGGLE_LINES.fetch_add(1, Ordering::Relaxed) < 3000 {
        log::info!("{message}");
    }
}

/// Swaps the sprite of a toggle to show its slot's choice.
unsafe fn paint_toggle(gui: *mut u8, slot: usize) {
    let set_bg: SetBgFn = std::mem::transmute(GUI_SETBG);
    let on = SLOT_MASK.load(Ordering::Relaxed) & (1 << slot) != 0;
    let sprite = if on { SPRITE_ON } else { SPRITE_OFF };
    set_bg(gui, 0, sprite.as_ptr(), read::<f32>(BG_MINUS_ONE), 0, -1);
}

/// The player's ship cards on the pre-combat screen (`PRECOMBAT_A`; `PRECOMBAT_B` are the enemy's), left to right:
/// (x, y, width, height). Positions are the float pairs at +0x128, sizes at +0x218.
unsafe fn card_elements(quiet: bool) -> Vec<(f32, f32, f32, f32)> {
    let head: usize = read(GUI_LIST);
    let mut out = Vec::new();
    let mut other = Vec::new();
    let mut node: usize = read(head);
    for _ in 0..600 {
        if node == 0 || node == head {
            break;
        }
        let element: usize = read(node + 0x10);
        if element != 0 && read::<i32>(element + OFF_ACTIVE) != 0 {
            let name = cstr(element + GUI_NAME);
            let entry = (read::<f32>(element + 0x128), read::<f32>(element + 0x12c), read::<f32>(element + 0x218), read::<f32>(element + 0x21c));
            if name == "PRECOMBAT_A" {
                out.push(entry);
            } else if !quiet && (name == "PRECOMBAT_B" || name == "MINIFIG_NUMBER" || name == "MES_APP") {
                other.push(format!("{name} ({:.0},{:.0} {:.0}x{:.0})", entry.0, entry.1, entry.2, entry.3));
            }
        }
        node = read(node);
    }
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    if !quiet {
        toggle_line(format!(
        "ALLY cards: {} ship cards {:?} | others {}",
        out.len(),
        out.iter().map(|c| format!("({:.0},{:.0} {:.0}x{:.0})", c.0, c.1, c.2, c.3)).collect::<Vec<_>>(),
        other.join(" "),
        ));
    }
    out
}

/// Builds one GUI element the way the game builds PRECOMBAT_OK (sequence read from `0x1402d7620`).
unsafe fn make_gui(name: &[u8]) -> *mut u8 {
    let root: usize = read(GUI_ROOT);
    if root == 0 {
        return std::ptr::null_mut();
    }
    let new: NewFn = std::mem::transmute(GUI_NEW);
    let ctor: CtorFn = std::mem::transmute(GUI_CTOR);
    let mem = new(0xe70);
    if mem.is_null() {
        return mem;
    }
    let gui = ctor(mem, 0);
    if gui.is_null() {
        return gui;
    }
    let vtable: usize = read(gui as usize);
    let init: GuiInitFn = std::mem::transmute(read::<usize>(vtable + 0x2c0));
    let scale = read::<f32>(INIT_SCALE);
    let strings = INIT_STR.map(|a| a as *const u8);
    init(
        gui,
        name.as_ptr(),
        read::<f32>(INIT_F2),
        read::<f32>(INIT_F3),
        read::<i32>(INIT_DIM_A) as f32 * scale,
        read::<i32>(INIT_DIM_B) as f32 * scale,
        strings[0],
        strings[1],
        strings[2],
        strings[3],
        strings[4],
        read::<f32>(INIT_ONE),
    );
    let node_vtable: usize = read(root);
    let add: NodeAddFn = std::mem::transmute(read::<usize>(node_vtable + 0x50));
    add(root as *mut u8, gui, 0, 0);
    gui
}

/// Gets the element called `name`: an active one the game still lists, else our kept (hidden) one, else a new one.
/// Returns (gui, created).
unsafe fn get_or_make(name: &[u8], kept_slot: &std::sync::atomic::AtomicUsize) -> Option<(usize, bool)> {
    let find: FindFn = std::mem::transmute(GUI_FIND);
    let wanted = String::from_utf8_lossy(&name[..name.len() - 1]).to_string();
    let mut gui = find(name.as_ptr());
    if gui == 0 {
        let kept = kept_slot.load(Ordering::Relaxed);
        if kept != 0 && cstr(kept + GUI_NAME) == wanted {
            (kept as *mut i32).byte_add(OFF_ACTIVE).write_unaligned(2);
            gui = kept;
        }
    }
    if gui != 0 {
        return Some((gui, false));
    }
    let ptr = make_gui(name);
    if ptr.is_null() {
        return None;
    }
    Some((ptr as usize, true))
}

/// The ship creature of each player card (`PRECOMBAT_A`), left to right: the card's `+0xbc0` is a Property pointer, `GetPropertyLinkedObject` gives the ship.
unsafe fn card_ship_list() -> Vec<(f32, usize)> {
    let head: usize = read(GUI_LIST);
    let mut out = Vec::new();
    let mut node: usize = read(head);
    for _ in 0..600 {
        if node == 0 || node == head {
            break;
        }
        let element: usize = read(node + 0x10);
        if element != 0 && read::<i32>(element + OFF_ACTIVE) != 0 && cstr(element + GUI_NAME) == "PRECOMBAT_A" {
            out.push((read::<f32>(element + 0x128), plane_loadouts::ship_of_link(read::<usize>(element + 0xbc0))));
        }
        node = read(node);
    }
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// Sets (or removes) the text beside the plane button of `slot`.
fn set_plane_label(slot: usize, label: Option<(f32, f32, f32, String)>) {
    if let Ok(mut labels) = PLANE_LABELS.lock() {
        labels.retain(|l| l.0 != slot);
        if let Some((x, y, size, text)) = label {
            let mut bytes: Vec<u8> = text.bytes().filter(|b| b.is_ascii() && *b != 0).collect();
            bytes.push(0);
            labels.push((slot, x, y, size, bytes));
        }
    }
}

/// Paints the plane button with the sprite of its ship's current choice.
unsafe fn paint_loadout(gui: *mut u8, ship: usize, craft: usize) {
    let set_bg: SetBgFn = std::mem::transmute(GUI_SETBG);
    let sprite = if plane_loadouts::choice(ship, craft).is_some() { plane_loadouts::SPRITE_CHOSEN } else { plane_loadouts::SPRITE_DEFAULT };
    set_bg(gui, 0, sprite.as_ptr(), read::<f32>(BG_MINUS_ONE), 0, -1);
}

/// Places, shows or hides the button of plane type `craft` of card `card` (1-based); true when it is shown.
unsafe fn place_loadout(card: usize, craft: usize, ship: Option<usize>, pos: [f32; 2], size: f32) -> bool {
    let slot = craft * 9 + card; // 1-based button number
    let kept_slot = &LOADOUT_GUIS[slot - 1];
    let name = LOADOUT_NAMES[slot - 1];
    let shown = ship.filter(|&s| plane_loadouts::has_craft(s, craft) && (!plane_loadouts::available_arms(s, craft).is_empty() || plane_loadouts::choice(s, craft).is_some()));
    if shown.is_none() && craft == 0 {
        if let Some(s) = ship {
            toggle_line(format!("PLANE button card {card} hidden: {}", plane_loadouts::describe(s)));
        } else if card <= 3 {
            toggle_line(format!("PLANE button card {card} hidden: no ship link on this card"));
        }
    }
    let Some(ship) = shown else {
        set_plane_label(slot, None);
        let kept = kept_slot.load(Ordering::Relaxed);
        if kept != 0 && cstr(kept + GUI_NAME).as_bytes() == &name[..name.len() - 1] {
            (kept as *mut i32).byte_add(OFF_ACTIVE).write_unaligned(0);
        }
        return false;
    };
    let Some((gui, created)) = get_or_make(name, kept_slot) else {
        toggle_line(format!("PLANE button {slot}: not created"));
        return false;
    };
    let ptr = gui as *mut u8;
    // a button the battle start switched off may still be found in the game's list (a RETRY in Shipworks): switch it on again
    if read::<i32>(gui + OFF_ACTIVE) == 0 {
        (ptr as *mut i32).byte_add(OFF_ACTIVE).write_unaligned(2);
    }
    paint_loadout(ptr, ship, craft);
    if created {
        let vt: usize = read(gui);
        let refresh: ResetFn = std::mem::transmute(read::<usize>(vt + 0x88));
        refresh(ptr);
        let reset: ResetFn = std::mem::transmute(GUI_RESET);
        reset(ptr);
        kept_slot.store(gui, Ordering::Relaxed);
    }
    let set_pos: SetPosFn = std::mem::transmute(GUI_SETPOS);
    set_pos(ptr, pos.as_ptr());
    (ptr as *mut f32).byte_add(0x218).write_unaligned(size);
    (ptr as *mut f32).byte_add(0x21c).write_unaligned(size);
    // a moved button must rebuild its picture at the new place (a button dragged to another card showed its icon offset)
    {
        let vt: usize = read(gui);
        let refresh: ResetFn = std::mem::transmute(read::<usize>(vt + 0x88));
        refresh(ptr);
    }
    set_plane_label(slot, Some((pos[0], pos[1], size, plane_loadouts::label_for(craft, plane_loadouts::choice(ship, craft)))));
    toggle_line(format!("PLANE button {slot}: ship {ship:#x} {} at ({:.0},{:.0}) size {size:.0}, choice {:?}", plane_loadouts::CRAFTS[craft].1, pos[0], pos[1], plane_loadouts::choice(ship, craft).map(|a| plane_loadouts::ARMS[a].key)));
    true
}

/// A signature of the card layout: the number of cards and their sorted x positions.
fn card_signature(cards: &[(f32, f32, f32, f32)]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ cards.len() as u64;
    for c in cards {
        h = (h ^ c.0.to_bits() as u64).wrapping_mul(0x100_0000_01b3);
    }
    if PLANE_BUTTONS.load(Ordering::Relaxed) {
        // two cards swapping places keep the same x positions: the ship behind each card counts too
        for (_, ship) in unsafe { card_ship_list() } {
            h = (h ^ ship as u64).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

/// Places (creating on first use) the ALLY buttons and the slot-1 marker beside the ship numbers. Slot n is the
/// n-th card from the left (`precombatOk` orders the squad by card x), so after the player shuffles the cards
/// this is called again and every button goes to the card now standing in its slot.
unsafe fn ensure_toggles() {
    let mut cards = card_elements(false);
    let signature = card_signature(&cards);
    // cards stacked on one spot (not laid out yet): fall back to a grid read off the screenshots
    if cards.len() > 1 && cards.iter().all(|c| (c.0 - cards[0].0).abs() < 1.0) {
        for (i, c) in cards.iter_mut().enumerate() {
            *c = (GRID_X0 + GRID_STEP * i as f32, GRID_Y - TOGGLE_DY, c.2, c.3);
        }
        toggle_line("ALLY cards report no layout: using the screenshot grid".to_string());
    }
    let set_pos: SetPosFn = std::mem::transmute(GUI_SETPOS);
    let mut layout = Vec::new();
    // slot 1 (the flagship, always the player's ship) gets the marker, slots 2.. the toggles
    for slot in 1..=9usize {
        let Some(&(x, _y, _, _)) = cards.get(slot - 1) else {
            if slot == 1 {
                MARKER_SHOW.store(false, Ordering::Relaxed);
                continue;
            }
            // no such ship on this screen: hide a button kept from an earlier build
            let kept = TOGGLE_GUIS[slot - 2].load(Ordering::Relaxed);
            if kept != 0 && cstr(kept + GUI_NAME) == format!("ALLY_TOGGLE_{slot}") {
                (kept as *mut i32).byte_add(OFF_ACTIVE).write_unaligned(0);
            }
            continue;
        };
        // room between this card's number and the next card's: the button sits in the middle of the free span
        // when the cards are close together (the Grach pair is only 104 apart), else a fixed step right of the number
        let gap = cards.get(slot).map(|n| n.0 - x);
        let dx = gap.map_or(TOGGLE_DX, |g| TOGGLE_DX.min(g * 0.5));
        let size = gap.map_or(TOGGLE_SIZE, |g| (g - NUMBER_WIDTH).clamp(TOGGLE_MIN_SIZE, TOGGLE_SIZE));
        if slot == 1 {
            // the slot-1 label is text drawn from a render seam (`draw_marker`), not a GUI element: it only needs the spot
            MARKER_X.store((x + dx + MARKER_DX).to_bits(), Ordering::Relaxed);
            MARKER_SHOW.store(true, Ordering::Relaxed);
            layout.push(format!("1@{x:.0}+{dx:.0}/text"));
            continue;
        }
        let (name, kept_slot) = (TOGGLE_NAMES[slot - 2], &TOGGLE_GUIS[slot - 2]);
        let Some((gui, created)) = get_or_make(name, kept_slot) else {
            toggle_line(format!("ALLY toggle {slot}: not created"));
            return;
        };
        let ptr = gui as *mut u8;
        if created {
            paint_toggle(ptr, slot);
            let vt: usize = read(gui);
            let refresh: ResetFn = std::mem::transmute(read::<usize>(vt + 0x88));
            refresh(ptr);
            let reset: ResetFn = std::mem::transmute(GUI_RESET);
            reset(ptr);
            kept_slot.store(gui, Ordering::Relaxed);
        } else {
            paint_toggle(ptr, slot);
        }
        // the numbers sit at one height for every card, so the buttons do too (the card height varies per ship class)
        let pos = [x + dx, GRID_Y];
        set_pos(ptr, pos.as_ptr());
        // hit box = icon size (creation leaves the 274 x 154 screen-based size from the initializer)
        (ptr as *mut f32).byte_add(0x218).write_unaligned(size);
        (ptr as *mut f32).byte_add(0x21c).write_unaligned(size);
        layout.push(format!("{slot}@{:.0}+{:.0}/{:.0}", x, dx, size));
    }
    if PLANE_BUTTONS.load(Ordering::Relaxed) {
        if !PRECOMBAT_LIVE.load(Ordering::Relaxed) {
            // a fresh visit of the screen: forget the old elements and build new ones. A RETRY in Shipworks rebuilds the
            // screen; the old element is still in the game's list and active, yet is not drawn (rel.41 screenshot).
            hide_plane_buttons(&[]);
            for kept in &LOADOUT_GUIS {
                kept.store(0, Ordering::Relaxed);
            }
        }
        let ships = card_ship_list();
        let mut shown: Vec<usize> = Vec::new();
        for slot in 1..=9usize {
            let Some(&(x, _y, _, _)) = cards.get(slot - 1) else {
                for craft in 0..plane_loadouts::CRAFTS.len() {
                    place_loadout(slot, craft, None, [0.0, 0.0], 0.0);
                }
                continue;
            };
            let gap = cards.get(slot).map(|n| n.0 - x);
            let dx = gap.map_or(TOGGLE_DX, |g| TOGGLE_DX.min(g * 0.5));
            let size = gap.map_or(TOGGLE_SIZE, |g| (g - NUMBER_WIDTH).clamp(TOGGLE_MIN_SIZE, TOGGLE_SIZE));
            // plane buttons per card: T-7 under the ALLY button, LA-29 beside it
            let psize = size; // the sprite is drawn at its own size: scaled down it is blurry
            for craft in 0..plane_loadouts::CRAFTS.len() {
                // below the ALLY button, in the clickable (blue) part of the screen; the banner above takes no clicks
                // T-7 under the ALLY button; LA-29 in the ALLY row, right of it (under the button it would sit on the ship card, which takes the click)
                let pos = if craft == 0 { [x + dx, GRID_Y + size + LOADOUT_GAP] } else { [x + dx + size + LOADOUT_GAP * 2.0, GRID_Y] };
                if place_loadout(slot, craft, ships.get(slot - 1).map(|s| s.1), pos, psize) {
                    shown.push(craft * 9 + slot);
                }
            }
        }
        hide_plane_buttons(&shown);
    }
    PLACED_SIG.store(signature, Ordering::Relaxed);
    if !PRECOMBAT_LIVE.swap(true, Ordering::Relaxed) {
        plane_loadouts::clear(); // a fresh visit of the screen: no choice from an earlier one
        ENTRY_MS.store(now_ms(), Ordering::Relaxed); // a fresh entry: the roller covers are about to play
        ROLL_SEEN.store(false, Ordering::Relaxed);
    }
    toggle_line(format!("ALLY placed (slot@card x+dx/size): {}", layout.join(" ")));
}

/// Per-frame check while the pre-combat screen is up (called from the game thread by two hooks): when the cards
/// have been standing still for a moment and not where the buttons were placed for, place the buttons again.
/// Waiting for the cards to settle keeps the buttons from jumping while a card is being dragged.
unsafe fn precombat_tick(source: usize) {
    if !PRECOMBAT_LIVE.load(Ordering::Relaxed) {
        return;
    }
    if TICKS_SEEN[source].fetch_add(1, Ordering::Relaxed) == 0 {
        toggle_line(format!("ALLY precombat tick: first call from hook {source} (0 = input task, 1 = battle manager)"));
    }
    let cards = card_elements(true);
    if cards.is_empty() {
        return;
    }
    let sig = card_signature(&cards);
    let now = now_ms();
    if sig != TICK_SIG.swap(sig, Ordering::Relaxed) {
        TICK_CHANGED_MS.store(now, Ordering::Relaxed);
        if diag::DEBUG && TICK_LOGS.fetch_add(1, Ordering::Relaxed) < 4000 {
            log::info!(
                "ALLY card layout moving: {:?}",
                cards.iter().map(|c| format!("{:.1}", c.0)).collect::<Vec<_>>()
            );
        }
        return;
    }
    if sig != PLACED_SIG.load(Ordering::Relaxed) && now.wrapping_sub(TICK_CHANGED_MS.load(Ordering::Relaxed)) >= SETTLE_MS {
        toggle_line("ALLY cards settled: placing the buttons again".to_string());
        ensure_toggles();
    }
}

/// Draws the slot-1 label with the game's own text code (as the elevation readout does): the font is a shared
/// object, so its style is saved and put back.
unsafe extern "C" fn draw_marker_b22() {
    draw_marker(22);
}
unsafe extern "C" fn draw_marker_b23() {
    draw_marker(23);
}
unsafe extern "C" fn draw_marker_b24() {
    draw_marker(24);
}

/// The metal roller covers that play when the pre-combat screen opens (`ROLL_DESK`, `ROLL_TABLET`, `ROLL_TASK`,
/// `ROLL_ALARM`: GUI elements drawn above the buttons; a render seam would put our text on top of them). Seen in the
/// v0.11.8 log: `ROLL_DESK` (1155 x 843) exists from the build at y -1020 (away), slides down to y -20 at about
/// +1.5 s (covering the screen), back up at about +3.6 s, and is gone at about +3.9 s. The label waits for the whole
/// cycle: hidden while `ROLL_DESK` exists, and until it has been seen and gone (or `ROLL_TIMEOUT_MS` after entry, if
/// it never shows up). Later rebuilds (ship swaps) have no cover. Returns true = do not draw. State changes are logged.
unsafe fn doors_closed() -> bool {
    let find: FindFn = std::mem::transmute(GUI_FIND);
    let desk = find(b"ROLL_DESK\0".as_ptr());
    let since_entry = now_ms().wrapping_sub(ENTRY_MS.load(Ordering::Relaxed));
    if desk != 0 {
        ROLL_SEEN.store(true, Ordering::Relaxed);
    }
    let hold = desk != 0 || (!ROLL_SEEN.load(Ordering::Relaxed) && since_entry < ROLL_TIMEOUT_MS);
    let y = if desk == 0 { f32::NEG_INFINITY } else { read::<f32>(desk + 0x12c) };
    let state = (hold as u32) | (((y / 50.0) as i32 as u32) << 1);
    if state != DOOR_STATE.swap(state, Ordering::Relaxed) && DOOR_LOGS.fetch_add(1, Ordering::Relaxed) < 400 {
        toggle_line(format!("ALLY roller cover: ROLL_DESK y={y:.0} label held back={hold} t+{since_entry}ms"));
    }
    hold
}
static ROLL_SEEN: AtomicBool = AtomicBool::new(false);
/// A fresh entry without any ROLL_DESK shows the label after this long.
const ROLL_TIMEOUT_MS: u64 = 6000;
static DOOR_STATE: AtomicU32 = AtomicU32::new(0);
static DOOR_LOGS: AtomicU32 = AtomicU32::new(0);

unsafe fn draw_marker(seam: u32) {
    draw_marker_text(seam);
    draw_plane_labels(seam);
}

/// The plane buttons' labels, drawn the same way as the slot-1 text and under the same gates.
unsafe fn draw_plane_labels(seam: u32) {
    let _hook = diag::scope("draw_plane_labels");
    if seam != MARKER_SEAM_SEL.load(Ordering::Relaxed) || !PRECOMBAT_LIVE.load(Ordering::Relaxed) || doors_closed() {
        return;
    }
    let Ok(labels) = PLANE_LABELS.try_lock().map(|l| l.clone()) else { return };
    if labels.is_empty() {
        return;
    }
    let manager: *mut u8 = read(MEDIA_RES);
    if manager.is_null() {
        return;
    }
    let get_font: GetFontFn = std::mem::transmute(GET_FONT);
    let font = get_font(manager, LABEL_FONT.as_ptr());
    if font.is_null() {
        return;
    }
    let base = font as usize;
    let (old_scale, old_color, old_blend): (f32, u32, i32) = (read(base + 0x100c), read(base + 0x1020), read(base + 0x1028));
    let style: StyleFn = std::mem::transmute(STYLE_FONT);
    style(LABEL_FONT.as_ptr(), 1.0, 2, LABEL_COLOR);
    let render: RenderTextFn = std::mem::transmute(FONT_RENDER);
    for (_, bx, by, size, text) in labels {
        // right of the round button, vertically centred on it
        // whole pixels only: a bitmap font drawn between pixels is blurred
        let (x, y) = ((960.0 + bx + size * 0.5 + 6.0).round(), (540.0 + by - 8.0).round());
        render(font, x, y, 0, text.as_ptr() as *const _, std::ptr::null_mut(), std::ptr::null_mut(), 0, 0.0); // align 0 = left edge at x
    }
    style(LABEL_FONT.as_ptr(), old_scale, old_blend, old_color);
}

unsafe fn draw_marker_text(seam: u32) {
    let _hook = diag::scope("draw_marker");
    if PRECOMBAT_LIVE.load(Ordering::Relaxed) && seam == MARKER_SEAM_SEL.load(Ordering::Relaxed) && doors_closed() {
        return;
    }
    if seam != MARKER_SEAM_SEL.load(Ordering::Relaxed) || now_ms().wrapping_sub(ENTRY_MS.load(Ordering::Relaxed)) < MARKER_DELAY_MS.load(Ordering::Relaxed) {
        return;
    }
    if !PRECOMBAT_LIVE.load(Ordering::Relaxed) || !MARKER_SHOW.load(Ordering::Relaxed) {
        return;
    }
    let Ok(text) = MARKER_TEXT.lock().map(|t| t.clone()) else { return };
    if text.len() <= 1 {
        return;
    }
    let manager: *mut u8 = read(MEDIA_RES);
    if manager.is_null() {
        return;
    }
    let get_font: GetFontFn = std::mem::transmute(GET_FONT);
    let font = get_font(manager, MARKER_FONT.as_ptr());
    if font.is_null() {
        return;
    }
    let base = font as usize;
    let (old_scale, old_color, old_blend): (f32, u32, i32) = (read(base + 0x100c), read(base + 0x1020), read(base + 0x1028));
    let style: StyleFn = std::mem::transmute(STYLE_FONT);
    style(MARKER_FONT.as_ptr(), MARKER_SCALE, 2, MARKER_COLOR);
    // GUI units have the origin in the middle of the 1920 x 1080 frame; text coordinates start at the top left
    // whole pixels (a bitmap font between pixels is blurred); PLAYER_NUDGE brings it closer to the ship number
    let x = (960.0 + f32::from_bits(MARKER_X.load(Ordering::Relaxed)) + PLAYER_NUDGE).round();
    let y = (540.0 + GRID_Y - 11.0 * MARKER_SCALE).round();
    let render: RenderTextFn = std::mem::transmute(FONT_RENDER);
    render(font, x, y, 2, text.as_ptr() as *const _, std::ptr::null_mut(), std::ptr::null_mut(), 0, 0.0);
    style(MARKER_FONT.as_ptr(), old_scale, old_blend, old_color);
    if MARKER_DRAWS.fetch_add(1, Ordering::Relaxed) == 0 {
        toggle_line(format!("ALLY marker text first drawn at ({x:.0},{y:.0})"));
    }
}
const MARKER_SCALE: f32 = 1.0;
/// The slot-1 label sat too far from the number "1": shifted left by this many GUI units.
const PLAYER_NUDGE: f32 = -40.0;
const MARKER_COLOR: u32 = 0xffe0_e0db;
/// The ammo picker's own label colour (notes section 15: markup `{font=flash_large_shadow}{color=4293514025}`); the shadow is part of that font.
const LABEL_COLOR: u32 = 0xffe9_d329;

unsafe extern "C" fn on_input_frame() {
    let _hook = diag::scope("on_input_frame");
    precombat_tick(0);
}

/// Switches off every listed `PLANE_LOADOUT_n` element whose number is not in `keep` (a stale one from an earlier card order).
unsafe fn hide_plane_buttons(keep: &[usize]) {
    let head: usize = read(GUI_LIST);
    let mut node: usize = read(head);
    for _ in 0..600 {
        if node == 0 || node == head {
            break;
        }
        let element: usize = read(node + 0x10);
        if element != 0 && read::<i32>(element + OFF_ACTIVE) != 0 {
            if let Some(n) = cstr(element + GUI_NAME).strip_prefix("PLANE_ARM_").and_then(|n| n.parse::<usize>().ok()) {
                if !keep.contains(&n) {
                    (element as *mut i32).byte_add(OFF_ACTIVE).write_unaligned(0);
                }
            }
        }
        node = read(node);
    }
}

/// Hides the toggles (active flag 0, as the game does for its own unused elements); the game never frees ours.
unsafe fn hide_toggles(why: &str) {
    PRECOMBAT_LIVE.store(false, Ordering::Relaxed);
    hide_plane_buttons(&[]);
    if let Ok(mut labels) = PLANE_LABELS.lock() {
        labels.clear();
    }
    let mut hidden = 0;
    for (i, name) in TOGGLE_NAMES.iter().enumerate() {
        let kept = TOGGLE_GUIS[i].load(Ordering::Relaxed);
        if kept != 0 && cstr(kept + GUI_NAME).as_bytes() == &name[..name.len() - 1] && read::<i32>(kept + OFF_ACTIVE) != 0 {
            (kept as *mut i32).byte_add(OFF_ACTIVE).write_unaligned(0);
            hidden += 1;
        }
    }
    for (i, name) in LOADOUT_NAMES.iter().enumerate() {
        let kept = LOADOUT_GUIS[i].load(Ordering::Relaxed);
        if kept != 0 && cstr(kept + GUI_NAME).as_bytes() == &name[..name.len() - 1] && read::<i32>(kept + OFF_ACTIVE) != 0 {
            (kept as *mut i32).byte_add(OFF_ACTIVE).write_unaligned(0);
        }
    }
    if hidden > 0 {
        toggle_line(format!("ALLY toggles hidden ({why}): {hidden}"));
    }
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

unsafe extern "C" fn on_ammo_press() {
    let _hook = diag::scope("on_ammo_press");
    inspect_gui("ammoSelectButtonPress");
}

/// Logs every active GUI element: name, class, position, size, value fields. Throttled, at most 80 dumps.
unsafe fn inspect_gui(trigger: &str) {
    let now = now_ms();
    if now.wrapping_sub(INSPECT_MS.load(Ordering::Relaxed)) < 700 || INSPECT_DUMPS.load(Ordering::Relaxed) >= 80 {
        return;
    }
    INSPECT_MS.store(now, Ordering::Relaxed);
    let dump = INSPECT_DUMPS.fetch_add(1, Ordering::Relaxed);
    let head: usize = read(GUI_LIST);
    let mut lines = Vec::new();
    let mut node: usize = read(head);
    for _ in 0..400 {
        if node == 0 || node == head {
            break;
        }
        let e: usize = read(node + 0x10);
        if e != 0 && read::<i32>(e + OFF_ACTIVE) != 0 {
            lines.push(format!(
                "{}[{}] act={} pos=({:.0},{:.0}) size={:.0}x{:.0} vt={:#x} c00={} link={:#x}",
                cstr(e + GUI_NAME),
                cstr(e + 8),
                read::<i32>(e + OFF_ACTIVE),
                read::<f32>(e + 0x128),
                read::<f32>(e + 0x12c),
                read::<f32>(e + 0x218),
                read::<f32>(e + 0x21c),
                read::<usize>(e).wrapping_sub(IMAGE_BASE),
                read::<i32>(e + 0xc00),
                read::<usize>(e + 0xbc0),
            ));
        }
        node = read(node);
    }
    log::info!("GUI dump #{dump} [{trigger}] {} elements: {}", lines.len(), lines.join(" ; "));
}

/// Called at the entry of ClickFunc (RCX ctx, RDX gui, R8 button, R9 id); true = the click was ours, skip the game.
unsafe extern "C" fn on_click_pre(_ctx: usize, gui: usize, button: usize, _id: usize) -> bool {
    let _hook = diag::scope("on_click_pre");
    if gui == 0 {
        return false;
    }
    let name = cstr(gui + GUI_NAME);
    crumb!("CLICK {name} gui={gui:#x} button={button} id={_id:#x} {}", scene());
    restore_orders("click", false);
    diag::snapshot(&format!("click {name}"));
    if GUI_INSPECT.load(Ordering::Relaxed) {
        inspect_gui(&format!("click {name}"));
    }
    // the end-of-battle screen and the way out of the game: the summary of the battle that just ended
    // (its result code is still fresh here); the battle hook never sees a real fight end
    if name.starts_with("RETRY_") || name.starts_with("QUIT_") {
        emit_summary(f64::from_bits(SUM_LAST.load(Ordering::Relaxed)));
        return false;
    }
    if name == "PRECOMBAT_OK" || name == "PRECOMBAT_BACK" {
        if name == "PRECOMBAT_BACK" {
            PRECOMBAT_SEEN.store(false, Ordering::Relaxed);
        }
        hide_toggles(&name);
        return false;
    }
    if let Some(number) = name.strip_prefix("PLANE_ARM_").and_then(|n| n.parse::<usize>().ok()) {
        let now = now_ms();
        let (craft, card) = ((number.max(1) - 1) / 9, (number.max(1) - 1) % 9 + 1);
        let slot = number; // the label key
        if button as u32 == 1 && (1..=18).contains(&number) && now.wrapping_sub(LAST_TOGGLE_MS.load(Ordering::Relaxed)) > 350 {
            LAST_TOGGLE_MS.store(now, Ordering::Relaxed);
            if let Some(&(_, ship)) = card_ship_list().get(card - 1) {
                let next = plane_loadouts::next_choice(ship, craft);
                plane_loadouts::set_choice(ship, craft, next);
                paint_loadout(gui as *mut u8, ship, craft);
                if let Ok(mut labels) = PLANE_LABELS.lock() {
                    if let Some(l) = labels.iter_mut().find(|l| l.0 == slot) {
                        let mut bytes = plane_loadouts::label_for(craft, next).into_bytes();
                        bytes.push(0);
                        l.4 = bytes;
                    }
                }
                let vt: usize = read(gui);
                let refresh: ResetFn = std::mem::transmute(read::<usize>(vt + 0x88));
                refresh(gui as *mut u8);
                toggle_line(format!("PLANE button clicked: slot {slot} ship {ship:#x} -> {:?}", next.map(|a| plane_loadouts::ARMS[a].key)));
            }
        }
        return true;
    }
    let Some(slot) = name.strip_prefix("ALLY_TOGGLE_").and_then(|n| n.parse::<usize>().ok()) else {
        return false;
    };
    // the game delivers one click twice (press and release): one toggle per 350 ms
    let now = now_ms();
    if button as u32 == 1 && (2..=9).contains(&slot) && now.wrapping_sub(LAST_TOGGLE_MS.load(Ordering::Relaxed)) > 350 {
        LAST_TOGGLE_MS.store(now, Ordering::Relaxed);
        let mask = SLOT_MASK.fetch_xor(1 << slot, Ordering::Relaxed) ^ (1 << slot);
        paint_toggle(gui as *mut u8, slot);
        let vt: usize = read(gui);
        let refresh: ResetFn = std::mem::transmute(read::<usize>(vt + 0x88));
        refresh(gui as *mut u8);
        toggle_line(format!("ALLY toggle clicked: ship {slot} {}", if mask & (1 << slot) != 0 { "ON" } else { "OFF" }));
    }
    true
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

unsafe fn precombat_dump(which: &str) {
    crumb!("PRECOMBAT seam {which} {}", scene());
    restore_orders("pre-combat screen", false);
    diag::snapshot(&format!("precombat {which}"));
    // the screen before a fight: the previous battle is over, log its summary now if the battle hook never saw it end
    emit_summary(f64::from_bits(SUM_LAST.load(Ordering::Relaxed)));
    if which.starts_with("B2") && TOGGLE_ENABLED.load(Ordering::Relaxed) {
        crumb!("ensure_toggles: enter");
        ensure_toggles();
        crumb!("ensure_toggles: done");
    }
    if !LOG_PRECOMBAT.load(Ordering::Relaxed) {
        return;
    }
    let build = PRECOMBAT_BUILDS.fetch_add(1, Ordering::Relaxed);
    if build >= 24 && !diag::DEBUG {
        return;
    }
    let head: usize = read(GUI_LIST);
    let count: usize = read(GUI_COUNT);
    let mut names = Vec::new();
    let mut ok = 0usize;
    let mut node: usize = read(head);
    for _ in 0..600 {
        if node == 0 || node == head {
            break;
        }
        let element: usize = read(node + 0x10);
        if element != 0 {
            let name = cstr(element + GUI_NAME);
            if name == "PRECOMBAT_OK" {
                ok = element;
            }
            names.push(format!("{name}@{element:#x}{}", if read::<i32>(element + OFF_ACTIVE) != 0 { "" } else { "(off)" }));
        }
        node = read(node);
    }
    log::info!("PRECOMBAT build #{build} [{which}]: GUI count {count}, {} listed: {}", names.len(), names.join(" "));
    if ok != 0 {
        let floats: Vec<String> = (0x100..0x140).step_by(4).map(|o| format!("{o:#x}={:.1}", read::<f32>(ok + o))).collect();
        let sizes: Vec<String> = (0x210..0x224).step_by(4).map(|o| format!("{o:#x}={:.1}", read::<f32>(ok + o))).collect();
        log::info!(
            "PRECOMBAT OK button c={ok:#x} vtable_rva={:#x} active={} floats {} | {}",
            read::<usize>(ok).wrapping_sub(IMAGE_BASE),
            read::<i32>(ok + OFF_ACTIVE),
            floats.join(" "),
            sizes.join(" "),
        );
    }
}

/// A pointer that can be a live game object: user-space, 8-aligned, not a small integer.
fn plausible(p: usize) -> bool {
    p >= 0x10000 && p < 0x7fff_ffff_0000 && p % 8 == 0
}

/// A creature that is still part of the battle: plausible pointer, creature class code, active, not dead.
unsafe fn live_ship(p: usize) -> bool {
    plausible(p) && read::<u8>(p + 0x28) & 0x2f == 0x2f && read::<i32>(p + OFF_ACTIVE) != 0 && !matches!(read::<i32>(p + OFF_STATE), 6 | 7)
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
const OFF_FUEL: usize = 0x1638; // f64: combat fuel now (the battle burns it: 0x14007a210)
const OFF_FUEL_CAPACITY: usize = 0x1640; // f64
const OFF_FUEL_AT_START: usize = 0xbd0; // f32: written at spawn (0x1402672c0)

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
    // with no ally left the flagship still has to be followed (its leaving restores the retreat point)
    if allies.is_empty() && !AUTO_RETREAT.load(Ordering::Relaxed) {
        return;
    }
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
    if !AUTO_RETREAT.load(Ordering::Relaxed) {
        return;
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
            "ALLY SUMMARY v{}: {:.0} s, squad {} (flagship + {} others), enemies at start {}, result {result} ({}), flagship left alive: {}, retreat key used: {}, allies brought {} lost {}, ally: {ally}",
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
    if read::<u8>(node + 0x28) & 0x2f == 0x2f {
        return node;
    }
    let owner: usize = read(node + 0xb8);
    if owner != 0 && read::<u8>(owner + 0x28) & 0x2f == 0x2f {
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
    if cb == 0 || ce < cb || (ce - cb) / 0x18 > 4096 {
        return;
    }
    let mut best: Option<(f32, usize)> = None;
    for a in (cb..ce).step_by(0x18) {
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
static LOG_COLORS: AtomicBool = AtomicBool::new(false);
static TITLE_PROBE_T: Mutex<f64> = Mutex::new(-100.0);

/// Diagnostic (`log_colors`): every 2 s lists the GUI elements named "TITLE" (the combat hover callouts, notes
/// `battle-hud-callouts.md` section 5) with what they link to: gui+0x178 (a creature? its side +0xad4; a part? its owner
/// +0xb8), the flag +0xc08 that chooses lime / red, and +0xa8c. Shows whether a callout knows its ship.
unsafe fn title_probe() {
    let now = read::<f64>(CLOCK);
    let Ok(mut last) = TITLE_PROBE_T.try_lock() else { return };
    if now - *last < 2.0 {
        return;
    }
    *last = now;
    let head: usize = read(GUI_LIST);
    let mut node: usize = read(head);
    let mut shown = 0;
    for _ in 0..800 {
        if node == 0 || node == head || shown >= 8 {
            break;
        }
        let g: usize = read(node + 0x10);
        if g != 0 && plausible(g) && cstr(g + GUI_NAME) == "TITLE" {
            let linked: usize = read(g + 0x178);
            let class = if plausible(linked) { read::<u8>(linked + 0x28) } else { 0 };
            let owner: usize = if plausible(linked) { read(linked + 0xb8) } else { 0 };
            let side = |p: usize| if plausible(p) { read::<f32>(p + OFF_SIDE) } else { f32::NAN };
            log::info!(
                "TITLE probe: gui {g:#x} active {} flag(+0xc08) {} +0xa8c {:#x} linked(+0x178) {linked:#x} class {class:#x} side(linked) {} owner(+0xb8) {owner:#x} side(owner) {}",
                read::<i32>(g + OFF_ACTIVE),
                read::<i32>(g + 0xc08),
                read::<u32>(g + 0xa8c),
                side(linked),
                side(owner)
            );
            shown += 1;
        }
        node = read(node);
    }
}

unsafe extern "C" fn on_battle_tick() {
    if LOG_COLORS.load(Ordering::Relaxed) {
        title_probe();
        // rescanned every 5 s: sprites are created lazily, their handle is 0 until first use
        static NAMES_T: Mutex<f64> = Mutex::new(-100.0);
        if let Ok(mut t) = NAMES_T.try_lock() {
            let now = read::<f64>(CLOCK);
            if now - *t >= 5.0 {
                *t = now;
                // color_probe::scan_names() crashed the game (reads freed list nodes) and matched no sprite: off
            }
        }
    }
    plane_loadouts::verify(read::<f64>(CLOCK));
    plane_loadouts::tick();
    let _hook = diag::scope("on_battle_tick");
    precombat_tick(1);
    #[cfg(feature = "telemetry")]
    telemetry::tick();
    if !SPAWN_ALLY.load(Ordering::Relaxed) {
        return;
    }
    let ships = roster();
    restore_orders("battle tick", false);
    if !ARMED.load(Ordering::Relaxed) {
        // not an ally battle: touch nothing; just keep the roster-empty bookkeeping so a later battle starts clean
        if ships.is_empty() {
            PLAYER_SEEN.store(0, Ordering::Relaxed);
        }
        return;
    }
    track_ally(&ships);
    retreat_logic(&ships);
    ally_sound_events(&ships);
    summary_tick(&ships);
    if !ships.is_empty() {
        hide_toggles("battle running");
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
    if AMMO_ON.load(Ordering::Relaxed) {
        if let (Ok(allies), Ok(mut rolled)) = (ALLIES.lock(), AMMO_ROLLED.lock()) {
            for &a in allies.iter() {
                if !rolled.contains(&a) && ally_ammo::roll(a) {
                    rolled.push(a);
                }
            }
        }
    }
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
            SLOT_MASK.load(Ordering::Relaxed),
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
    let mask = SLOT_MASK.load(Ordering::Relaxed);
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
    // the first line of this mod in every log: testers must be able to show which build they run
    log::info!("{}", diag::banner());
    diag::install();
    if !GAME_IS_STEAM_1_163.load(Ordering::Relaxed) {
        log::error!("hf_ally: needs Steam 1.163, not starting");
        return false;
    }
    if GetModuleHandleA(std::ptr::null()) as usize != IMAGE_BASE {
        log::error!("hf_ally: unexpected image base, not starting");
        return false;
    }

    let path = "Modloader/config/ally.json";
    let config: Config = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
            log::error!("hf_ally: bad {path}: {error}; using defaults");
            Config::default()
        }),
        Err(_) => {
            let config = Config::default();
            if let Ok(text) = serde_json::to_string_pretty(&config) {
                let _ = std::fs::write(path, text);
            }
            config
        }
    };
    if !config.enabled {
        log::info!("hf_ally: disabled in {path}");
        return true;
    }
    // the debug build lifts the line budgets (a session never gets them back: the level name is always "?")
    let max_lines = if diag::DEBUG { config.max_lines.max(1_000_000) } else { config.max_lines };
    MAX_LINES.store(max_lines, Ordering::Relaxed);
    SPAWN_ALLY.store(config.spawn_ally, Ordering::Relaxed);
    MARKER_SEAM_SEL.store(config.marker_seam.clamp(22, 24), Ordering::Relaxed);
    MARKER_DELAY_MS.store(config.marker_delay_ms, Ordering::Relaxed);
    if let Ok(mut text) = MARKER_TEXT.lock() {
        // ASCII only; '@' is a line break for the game's text code
        *text = config.marker_text.bytes().filter(|b| (0x20..0x7f).contains(b) && *b != b'@').chain([0]).collect();
    }
    PROBE_LINES.store(config.probe_lines || diag::DEBUG, Ordering::Relaxed);
    #[cfg(feature = "telemetry")]
    telemetry::ENABLED.store(config.telemetry, Ordering::Relaxed);
    MAX_ALLIES.store(config.max_allies.max(1), Ordering::Relaxed);
    AMMO_ON.store(config.ally_ai_ammo, Ordering::Relaxed);
    log::info!(
        "hf_ally v{} config: {}",
        env!("CARGO_PKG_VERSION"),
        serde_json::to_string(&config).unwrap_or_default()
    );
    AUTO_RETREAT.store(config.auto_retreat, Ordering::Relaxed);
    FUEL_RETREAT_PERCENT.store(config.ally_fuel_retreat_percent.clamp(0.0, 100.0).to_bits(), Ordering::Relaxed);
    HANDOVER.store(config.flagship_leaves == "handover", Ordering::Relaxed);
    RETREAT_KEY.store(config.retreat_key, Ordering::Relaxed);
    ALLY_DELAY.store(config.ally_delay_seconds.max(0.0).to_bits(), Ordering::Relaxed);

    match prepare(&config) {
        Ok(()) => {
            log::info!("hf_ally v{}: hooks installed (spawn log, creature log {}, ally {})", env!("CARGO_PKG_VERSION"), config.log_creatures, config.spawn_ally);
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
    if config.log_creatures && found != ADD_PROLOGUE {
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

    if config.ally_sounds && config.spawn_ally {
        let found = std::slice::from_raw_parts(PLAY_SFX as *const u8, PLAY_SFX_PROLOGUE.len());
        if found == PLAY_SFX_PROLOGUE {
            SOUNDS.store(true, Ordering::Relaxed);
        } else {
            log::warn!("hf_ally: playSFXSound_ bytes differ ({found:02x?}), ally sounds off");
        }
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
    if config.log_creatures {
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
    if config.log_precombat || config.ally_toggle || diag::DEBUG {
        HAVE_PRECOMBAT_HOOK.store(true, Ordering::Relaxed);
        let sites: [(usize, [u8; 5], *const ()); 3] = [
            (PRECOMBAT_SEAM, PRECOMBAT_SEAM_BYTES, on_precombat as *const ()),
            (PRECOMBAT_B1_SEAM, PRECOMBAT_B1_BYTES, on_precombat_b1 as *const ()),
            (PRECOMBAT_B2_SEAM, PRECOMBAT_B2_BYTES, on_precombat_b2 as *const ()),
        ];
        for (site, bytes, hook) in sites {
            let found = std::slice::from_raw_parts(site as *const u8, bytes.len());
            if found != bytes {
                return Err(format!("pre-combat seam {site:#x} has unexpected bytes {found:02x?}"));
            }
            let mut trampoline = Trampoline::new();
            trampoline.preserved_call(hook, &[], &[], ReturnType::None);
            trampoline.absolute_call(PRECOMBAT_SEAM_TARGET as *const ());
            trampoline.relative_jump(site + bytes.len());
            session.detour_trampoline(site, bytes.len(), trampoline).map_err(|e| e.to_string())?;
        }
    }
    if config.ally_toggle && config.spawn_ally {
        for (name, address, expected) in [
            ("gui new", GUI_NEW, &GUI_NEW_PROLOGUE[..]),
            ("gui ctor", GUI_CTOR, &GUI_CTOR_PROLOGUE[..]),
            ("gui setpos", GUI_SETPOS, &GUI_SETPOS_PROLOGUE[..]),
            ("gui setbg", GUI_SETBG, &GUI_SETBG_PROLOGUE[..]),
            ("gui reset", GUI_RESET, &GUI_RESET_PROLOGUE[..]),
            ("gui find", GUI_FIND, &GUI_FIND_PROLOGUE[..]),
        ] {
            let found = std::slice::from_raw_parts(address as *const u8, expected.len());
            if found != expected {
                return Err(format!("{name} at {address:#x} has unexpected bytes {found:02x?}: toggle not installed"));
            }
        }
    }
    if (config.ally_toggle && config.spawn_ally) || config.gui_inspect {
        let found = std::slice::from_raw_parts(CLICK_ENTRY as *const u8, CLICK_STOLEN.len());
        if found != CLICK_STOLEN {
            log::warn!("hf_ally: ClickFunc entry is already hooked by another mod (hf_probe hook_clicks?): no toggle, no GUI inspector");
        } else {
            // entry hook that can swallow the click: call the hook with the four arguments, `ret` when it says yes
            let mut code: Vec<u8> = vec![0x51, 0x52, 0x41, 0x50, 0x41, 0x51, 0x48, 0x83, 0xec, 0x28, 0x48, 0xb8];
            code.extend((on_click_pre as usize as u64).to_le_bytes());
            code.extend([0xff, 0xd0, 0x48, 0x83, 0xc4, 0x28, 0x84, 0xc0, 0x41, 0x59, 0x41, 0x58, 0x5a, 0x59]);
            code.extend([0x75, (CLICK_STOLEN.len() + 14) as u8]);
            code.extend(CLICK_STOLEN);
            code.extend([0xff, 0x25, 0, 0, 0, 0]);
            code.extend(((CLICK_ENTRY + CLICK_STOLEN.len()) as u64).to_le_bytes());
            code.push(0xc3);
            let mut trampoline = Trampoline::new();
            trampoline.bytes(&code);
            session
                .detour_trampoline(CLICK_ENTRY, CLICK_STOLEN.len(), trampoline)
                .map_err(|e| e.to_string())?;
            TOGGLE_ENABLED.store(config.ally_toggle && config.spawn_ally, Ordering::Relaxed);
        }
    }
    if config.ally_toggle && config.spawn_ally {
        for (seam, handler) in MARKER_SEAMS.iter().zip([draw_marker_b22 as *const (), draw_marker_b23 as *const (), draw_marker_b24 as *const ()]) {
            let bytes = std::slice::from_raw_parts(*seam as *const u8, 5);
            let target = (*seam + 5).wrapping_add(i32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as isize as usize);
            if bytes[0] == 0xe8 && target == FLUSH {
                let mut trampoline = Trampoline::new();
                trampoline.absolute_call(FLUSH as *const ());
                trampoline.preserved_call(handler, &[], &[], ReturnType::None);
                trampoline.relative_jump(*seam + 5);
                session.detour_trampoline(*seam, 5, trampoline).map_err(|e| e.to_string())?;
            } else {
                log::warn!("hf_ally: render seam {seam:#x} is not a call to the flush: not hooked");
            }
        }
    }
    if config.ally_toggle && config.spawn_ally {
        // per-frame input task (game thread): re-places the buttons after the cards are shuffled. Another mod on the
        // same entry (hf_voice) makes the bytes differ: then nothing ticks on this screen (the battle manager hook
        // does not run on it, seen in the v0.11.4-0.11.8 logs) and the buttons stay where the build put them.
        let found = std::slice::from_raw_parts(INPUT_TASK as *const u8, INPUT_TASK_PROLOGUE.len());
        if found == INPUT_TASK_PROLOGUE {
            // padded like hf_voice's: at a function entry RSP is 8 mod 16, patchy's register saver wants 0
            let mut trampoline = Trampoline::new();
            trampoline.bytes(&[0x48, 0x83, 0xec, 0x08]);
            trampoline.preserved_call(on_input_frame as *const (), &[], &[], ReturnType::None);
            trampoline.bytes(&[0x48, 0x83, 0xc4, 0x08]);
            trampoline.bytes(&INPUT_TASK_PROLOGUE);
            trampoline.relative_jump(INPUT_TASK + INPUT_TASK_PROLOGUE.len());
            session
                .detour_trampoline(INPUT_TASK, INPUT_TASK_PROLOGUE.len(), trampoline)
                .map_err(|e| e.to_string())?;
        } else {
            log::warn!("hf_ally: SomethingInputTask entry differs (another mod hooked it?): no per-frame card check from it");
        }
    }
    GUI_INSPECT.store(config.gui_inspect, Ordering::Relaxed);
    if config.gui_inspect {
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
    LOG_PRECOMBAT.store(config.log_precombat || diag::DEBUG, Ordering::Relaxed);
    SPARE_PRIZES.store(config.spare_prizes && config.spawn_ally, Ordering::Relaxed);
    if config.spare_prizes && config.spawn_ally {
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
    // independent of spawn_ally: the filter applies to every AI ship, the enemy's included
    if let Err(error) = intercept::install(&mut session, &config.intercept_ignore_oids) {
        log::warn!("hf_ally: {error}");
    }
    max_enemy::install(&mut session, config.max_enemy_ships);
    max_enemy::install_waves(&mut session, config.enemy_group_size, config.enemy_all_at_once);
    if config.plane_loadouts {
        match plane_loadouts::install(&mut session) {
            Ok(()) => {
                PLANE_BUTTONS.store(true, Ordering::Relaxed);
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
    if config.ally_damage_blue {
        ally_hud::install(&mut session, config.ally_damage_color);
    }
    if config.log_colors {
        color_probe::install(&mut session);
        LOG_COLORS.store(true, Ordering::Relaxed);
    }
    plane_loadouts::K13_ON.store(config.plane_k13, Ordering::Relaxed);
    if config.plane_bombers {
        plane_loadouts::install_bomber_gate(&mut session);
    }
    crash_fixes::install(&mut session, config.fix_plane_ai_crash, config.fix_merge_crash);
    if config.spawn_ally {
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
    }
    // the battle manager hook also carries the telemetry, so it is installed when either is on
    if config.spawn_ally || (cfg!(feature = "telemetry") && config.telemetry) {
        let battle = module.resolve_rva(BATTLE_RVA).map_err(|e| e.to_string())?;
        let found = std::slice::from_raw_parts(battle as *const u8, BATTLE_PROLOGUE.len());
        if found != BATTLE_PROLOGUE {
            return Err(format!("battle manager at {battle:#x} has unexpected bytes {found:02x?}"));
        }
        session
            .patch_call(
                battle,
                on_battle_tick as *const (),
                BATTLE_PROLOGUE.len(),
                true,
                ReturnType::None,
            )
            .map_err(|e| e.to_string())?;
    }
    #[cfg(feature = "telemetry")]
    if config.telemetry {
        telemetry::install_dead_node_hooks(&mut session)?;
    }
    session.install_permanently().map_err(|e| e.to_string())
}
