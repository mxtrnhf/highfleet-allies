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

mod ally_ammo;
mod ally_hud;
mod ally_panels;
mod ship_switch;
mod ally_reticles;
mod bomber;
mod bomber_math;
mod debug_overlay;
mod debug_state;
mod diag;
mod enemy_waves;
mod honor_share;
mod intercept;
mod missile_guard;
mod native_layout;
mod plane_loadouts;

use std::{
    ffi::{c_char, CStr},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Mutex,
    },
};

use std::mem::{offset_of, size_of};

use native_layout::{Body, Creature, Gui, Link, Node};

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
const GUI_NAME: usize = offset_of!(Gui, identifier);
const GUI_LINK_TARGET: usize = offset_of!(Gui, gui_l1) + offset_of!(Link, target);
// --- GUI element recipe (read from the PRECOMBAT_OK block of 0x1402d7620, see precombat-screen.md) ---
const GUI_NEW: usize = 0x1_4033_4094; // operator new(size)
const GUI_NEW_PROLOGUE: [u8; 9] = [0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9];
const GUI_CTOR: usize = 0x1_4012_a910; // GUI_2(mem, 0)
const GUI_CTOR_PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08];
const GUI_SETPOS: usize = 0x1_4005_20b0; // setXposYpos(obj, Vec2*)
const GUI_SETPOS_PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x10, 0x57, 0x48, 0x81, 0xec, 0xb0];
const GUI_SETBG: usize = 0x1_4001_be50; // SetBackground_(gui, idx, sprite, f32, u8, i32)
const GUI_SETBG_PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10];
/// `setBodyOwnerString(sprite, name, asFileName)` 0x1402e2030: the native name setter `SetBackground_` runs on
/// the Sprite it has just constructed (see its decompilation). Applying it to our existing newest sprite changes
/// the sprite instead of appending another one; it frees/clears the old animation state natively.
const SET_SPRITE_NAME: usize = 0x1_402e_2030;
const SET_SPRITE_NAME_PROLOGUE: [u8; 20] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20,
];
/// `ObjectArray_DeleteAll(gui)` 0x14004bb10: calls each sprite's native deleting destructor (slot 0, the
/// `New(0x1a8)` pair) and sets the vector end back to its begin. Used only to normalize an element that
/// accumulated more than one background; safe in builder/input/tick callbacks, never from a render hook.
const OBJECT_ARRAY_DELETE_ALL: usize = 0x1_4004_bb10;
const OBJECT_ARRAY_DELETE_ALL_PROLOGUE: [u8; 11] = [0x48, 0x89, 0x5c, 0x24, 0x10, 0x56, 0x48, 0x83, 0xec, 0x20, 0x48];
const GUI_RESET: usize = 0x1_4015_cc30; // clears the +0xab8/+0xad8/+0xaf8/+0xb18 strings
const GUI_RESET_PROLOGUE: [u8; 9] = [0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9];
const GUI_FIND: usize = 0x1_401a_9170; // FindGuiElement(char*)
const GUI_FIND_PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10];
const GUI_ROOT: usize = 0x1_47fc_6f88; // pointer to the global node every GUI is added to
/// The element list is walked to its own count plus this slack, capped so a corrupt count cannot hang the
/// game thread. A walk that reaches the cap is reported as incomplete and is never used to hide our elements.
const GUI_SCAN_SLACK: usize = 16;
const GUI_SCAN_MIN: usize = 256;
const GUI_SCAN_MAX: usize = 4096;
/// Byte size of one `Sprite*` entry in the native `Body.sprites` vector.
const GUI_SPRITE_SLOT_SIZE: usize = 8;
/// Guards the native sprite destruction/load walks; our custom buttons normally own exactly one sprite.
const GUI_SPRITE_LIMIT: usize = 4096;
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

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Config {
    /// Master switch. When true, every gameplay feature is installed, including the wingman.
    /// When false, no gameplay hooks are installed; the crash reporter still loads.
    enabled: bool,
    /// Governed dive, native-aimed whole-payload release and pull-out for bomb-carrying allies.
    ally_bombers: bool,
    /// Game seconds the player's ship must have been in the battle before the ally arrives.
    ally_delay_seconds: f64,
    /// What to do when the flagship leaves the arena alive: "handover" (default) = the ally is promoted to your
    /// ship and the retreat point the flagship used up is put back, so the new flagship can still retreat;
    /// "retreat" = the ally is extracted with it and the battle ends as a retreat. An invalid value logs a
    /// warning and uses "handover".
    flagship_leaves: String,
    /// Virtual-key code of the "ally, retreat" key (0x77 = F8); 0 disables the key.
    retreat_key: u32,
    /// How many AI allies may fight at the same time. Ships switched on beyond that wait in the reserve and
    /// step in, in ship-number order, when an ally is destroyed or has left.
    max_allies: u32,
    /// 1/2 take the displayed top/bottom ally; TAB cycles deployed friendly ships.
    ship_switching: bool,
    /// Allies never keep a prize ship (the ships the player may capture and sell after the battle: the game flags
    /// them at creature +0xc19) as a target: they pick another enemy, or hold fire when only prizes are left.
    spare_prizes: bool,
    /// Let AI-run allies launch their own ship missiles (the native Zenith launch path). False = an ally
    /// controlled by the AI holds its missile racks; the ship you fly can still launch manually.
    /// Ship bombers always suppress launches during a bombing course.
    ally_zenith_autofire: bool,
    /// Allies roll the AI's AP / proximity shell doctrine once, like enemy ships do at spawn. Default on.
    ally_ai_ammo: bool,
    /// Damage popups of allied ships are drawn in this colour, as HEX: `"#00E4FF"` (`"00E4FF"` and `"0x00E4FF"` work
    /// too). A plain number is still accepted so configs from before v0.14.0 keep working (read as 0xRRGGBB).
    #[serde(deserialize_with = "deserialize_hex_color")]
    ally_damage_color: String,
    /// Draw a half-size ship panel for each ally beside the player's own panel while the ally is in the battle.
    ally_panels: bool,
    /// Ally target box size relative to the native 62 x 62 frame. 0 = off; positive values
    /// scale the box; negative or non-finite values fall back to 0.8.
    ally_reticle_scale: f32,
    /// Radio sounds for ally events: the retreat key, an ally joining the fight, an ally destroyed. Lines follow
    /// the game's language: Russian from `Media/Snd/RADIO`, English from `Media/Snd/RADIO_ENG`.
    ally_sounds: bool,
    /// An ally leaves the battle (same order as the retreat key) when its COMBAT fuel (the ship's own tank, the
    /// one the battle burns; not the map fuel of the fleet) falls to this percent. 0 = off. Allies only.
    ally_fuel_retreat_percent: f32,
    /// Share of the player's battle honor/XP given to every deployed ally: 0 = off, 0.5 = 50% (default),
    /// 1 = 100%. The player keeps the full award; each ally receives its own `share * player award`.
    ally_honor_share: f32,
    /// Pre-combat button that picks the armament of a carrier's planes (charged to the inventory). Default on.
    plane_loadouts: bool,
    /// Give a chosen bomb / rocket loadout a 37 mm gunpod and keep the plane in the fight after the payload is gone:
    /// once bombs and rockets are empty it switches to the interceptor mode and strafes with the gun until that is dry.
    /// Normal tactical battle only (`Combat.seria`; `stateSelectorAircraft_ == 4`), never the strategic bomber scene.
    /// Default on.
    extra_gunpod: bool,
    /// Offer the K-13 air-to-air loadout for carrier planes. Enabled in the player default preset.
    plane_k13: bool,
    plane_attack_delay: f32,
    plane_aa_range: f32,
    plane_aa_delay: f32,
    plane_aa_angle: f32,
    plane_aa_speed: f32,
    /// Enemy slots active at once (vanilla 3, 1..=40). `Battle_DeployFromReserve` uses this for the
    /// initial group and for the refill after the 30 s timer. The weight budget is `3 * group_size`.
    enemy_group_size: u32,
    /// Set the 30 s reinforcement timer to 0, so refills start immediately instead of after 30 s.
    /// Does not change the total list or the active group size.
    enemy_all_at_once: bool,
    /// Text drawn next to ship number 1 on the pre-combat screen (the flagship, always your own ship); "" = none.
    marker_text: String,
    /// Diagnostic switches grouped for the config file.
    logging: LoggingConfig,
    /// SIDE PATCH (not in master). `m_oid`s the AI never treats as a strategic launch to intercept in the missile-vs-ship
    /// arcade scene, e.g. `["MDL_MISSILE_03"]` (the R-9 Sprint) for a Sprint modded to be strat-launchable (its category has bit 0x8000000, so
    /// the CIWS and anti-air launchers of both sides would shoot at it). Copy the names from `m_oid=` in OL.seria. Empty = vanilla.
    intercept_ignore_oids: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            ally_bombers: false,
            ally_delay_seconds: 1.5,
            flagship_leaves: "handover".to_string(),
            retreat_key: 0x77,
            max_allies: 2,
            ship_switching: true,
            spare_prizes: true,
            ally_zenith_autofire: true,
            ally_ai_ammo: true,
            ally_damage_color: "#00E4FF".to_string(),
            ally_panels: true,
            ally_reticle_scale: 0.8,
            ally_sounds: true,
            ally_fuel_retreat_percent: 30.0,
            ally_honor_share: 0.5,
            plane_loadouts: true,
            extra_gunpod: true,
            plane_k13: true,
            plane_attack_delay: 1.0,
            plane_aa_range: 6000.0,
            plane_aa_delay: 1.0,
            plane_aa_angle: 0.35,
            plane_aa_speed: 1.5,
            enemy_group_size: 3,
            enemy_all_at_once: false,
            marker_text: "PLAYER".to_string(),
            logging: LoggingConfig::default(),
            intercept_ignore_oids: vec!["MDL_MISSILE_03".to_string()],
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct LoggingConfig {
    /// Runtime debug mode: megalog, watchdog, crash minidump and the verbose diagnostics.
    /// Off by default and independent of the cargo feature.
    debug_log: bool,
    /// Battle debug overlay: rings around every ship and its AI driver state above the hull.
    /// Off by default; independent of the ally reticles.
    debug_overlay: bool,
    /// Full GUI lists on clicks and native ammo-selector rebuilds (diagnostic, off by default).
    gui_inspect: bool,
    /// Creature add/remove log (diagnostic, off by default).
    log_creatures: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            debug_log: false,
            debug_overlay: false,
            gui_inspect: false,
            log_creatures: false,
        }
    }
}

/// Accept old flat logging keys; explicit nested settings take precedence.
fn parse_config(text: &str) -> Result<Config, serde_json::Error> {
    let mut value: serde_json::Value = serde_json::from_str(text)?;
    if let Some(root) = value.as_object_mut() {
        let mut legacy = serde_json::Map::new();
        for key in ["debug_log", "debug_overlay", "gui_inspect", "log_creatures"] {
            if let Some(setting) = root.remove(key) {
                legacy.insert(key.to_string(), setting);
            }
        }
        let logging = root
            .entry("logging")
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if let Some(logging) = logging.as_object_mut() {
            for (key, setting) in legacy {
                logging.entry(key).or_insert(setting);
            }
        }
    }
    serde_json::from_value(value)
}

/// Read `ally_damage_color` from the config: a HEX string (`"#00E4FF"`, `"00E4FF"` or `"0x00E4FF"`), or a plain
/// number from a pre-v0.14.0 config, kept as HEX so the file the mod writes shows the colour the player knows.
fn deserialize_hex_color<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum HexColor {
        Text(String),
        Number(u64),
    }
    Ok(match HexColor::deserialize(deserializer)? {
        HexColor::Text(text) => text,
        HexColor::Number(value) => format!("#{:06X}", value & 0x00ff_ffff),
    })
}

/// Parse a HEX colour (`#RRGGBB`, `RRGGBB` or `0xRRGGBB`; alphabet is case-insensitive; 8 digits are AARRGGBB and the
/// alpha is dropped) into the game's 0xRRGGBB value.
fn parse_hex_color(text: &str) -> Option<u32> {
    let hex = text.trim();
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    let hex = hex.strip_prefix("0x").or_else(|| hex.strip_prefix("0X")).unwrap_or(hex);
    if hex.len() != 6 && hex.len() != 8 {
        return None;
    }
    u32::from_str_radix(hex, 16).ok().map(|value| value & 0x00ff_ffff)
}

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
/// v0.9.2/0.9.3: the ally logic runs only in a battle that was entered through the pre-combat screen (the card
/// choices come from it). Re-engaged chases, interceptions and other battles play as vanilla.
static PRECOMBAT_SEEN: AtomicBool = AtomicBool::new(false);
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
        let armed = PRECOMBAT_SEEN.swap(false, Ordering::Relaxed);
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
    diag::snapshot(&format!("spawn caller_rva={:#x}", caller.wrapping_sub(IMAGE_BASE)));
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
    unsafe fn line(&mut self, tag: &str, now: f64) -> String {
        let ship = self.who;
        let pos = (read::<f32>(ship + OFF_POS_X), read::<f32>(ship + OFF_POS_Y));
        let dt = (now - self.last_t).max(0.001) as f32;
        let speed = ((pos.0 - self.last_pos.0).powi(2) + (pos.1 - self.last_pos.1).powi(2)).sqrt() / dt;
        self.last_pos = pos;
        self.last_t = now;
        let target: usize = read(ship + OFF_TARGET);
        // A target link alone does not establish a live creature. Non-creature or
        // stale targets have an unknown distance; never follow them for logging.
        let target_dist = if probe_ship_in_battle(target) {
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

/// Validates a cached diagnostic pointer against the current creature list before
/// inspecting the object. Readable memory or a nonzero active field is not identity.
///
/// # Safety
/// Call only from the battle hook on the game thread, while the native creature
/// list and its objects cannot be removed concurrently. No native ownership is taken.
unsafe fn probe_ship_in_battle(ship: usize) -> bool {
    if !plausible(ship) {
        return false;
    }
    let (Some(begin), Some(end)) = (diag::rd::<usize>(CREATURES_BEGIN), diag::rd::<usize>(CREATURES_END)) else {
        return false;
    };
    if begin < 0x10000 || end < begin {
        return false;
    }
    let list_bytes = end - begin;
    if !list_bytes.is_multiple_of(LINK_STRIDE) || list_bytes / LINK_STRIDE > CREATURE_LIST_LIMIT {
        return false;
    }
    for entry in (begin..end).step_by(LINK_STRIDE) {
        let Some(current) = diag::rd::<usize>(entry + 8) else { return false };
        if current != ship {
            continue;
        }
        // SAFETY: current list membership is established above, on the game thread.
        // Only borrowed scalar fields are read, using the existing guarded reader.
        return diag::rd::<u8>(ship + OFF_CODE).is_some_and(|class| class & 0x2f == 0x2f)
            && diag::rd::<i32>(ship + OFF_ACTIVE).is_some_and(|active| active != 0)
            && diag::rd::<i32>(ship + OFF_STATE).is_some_and(|state| !matches!(state, 6 | 7));
    }
    false
}

/// The enemy nearest to `from`: side below zero, in the level.
///
/// # Safety
/// `from` must be a current creature validated on the game thread by the caller.
unsafe fn nearest_enemy(from: usize) -> usize {
    let (cb, ce): (usize, usize) = (read(CREATURES_BEGIN), read(CREATURES_END));
    if cb < 0x10000 || ce < cb {
        return 0;
    }
    let list_bytes = ce - cb;
    if !list_bytes.is_multiple_of(LINK_STRIDE) || list_bytes / LINK_STRIDE > CREATURE_LIST_LIMIT {
        return 0;
    }
    let (fx, fy) = (read::<f32>(from + OFF_POS_X), read::<f32>(from + OFF_POS_Y));
    let mut best = (usize::MAX as f32, 0usize);
    for a in (cb..ce).step_by(LINK_STRIDE) {
        let Some(p) = diag::rd::<usize>(a + 8) else { return 0 };
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
    // every frame: count launches and shots of the ally and of its nearest enemy
    if !probe_ship_in_battle(ally) {
        return;
    }
    if let (Ok(mut a), Ok(mut e)) = (PROBE_ALLY.lock(), PROBE_ENEMY.lock()) {
        if a.who != ally {
            a.reset(ally, now);
            e.reset(0, now);
        }
        // Check membership first: never read an old enemy's fields to decide
        // whether that pointer still belongs to an enemy ship.
        if !probe_ship_in_battle(e.who) || !diag::rd::<f32>(e.who + OFF_SIDE).is_some_and(|side| side < 0.0) {
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

/// Set once the pre-combat click hook is installed. Without it the buttons cannot respond, so they are not created.
static PRECOMBAT_UI_READY: AtomicBool = AtomicBool::new(false);
/// Per-ship ally choice: bit n = ship number n (2..=9, as numbered on the pre-combat screen) may be the ally.
static SLOT_MASK: AtomicU32 = AtomicU32::new(1 << 2);
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
/// B23 is the layer-flush call in the battle-branch renderer (which also draws the pre-combat screen).
/// B24 (`0x140309897`) is deliberately unused: the game skips it when GLITCH is OFF. B22 sits under the
/// sliding doors, so it is not used either.
const MARKER_SEAM: usize = 0x1_4030_9408;
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
/// The card positions the buttons were last moved to (a change means "move them with the cards this frame").
static PLACED_LAYOUT_SIG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static TICK_SIG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static TICK_CHANGED_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static TICK_LOGS: AtomicU32 = AtomicU32::new(0);
static TICKS_SEEN: [AtomicU32; 2] = [AtomicU32::new(0), AtomicU32::new(0)];
/// The plane-armament button is only drawn when its hook is installed.
static PLANE_BUTTONS: AtomicBool = AtomicBool::new(false);
/// Text beside each plane button, drawn from the render seam: (slot, x, y in GUI units, size, text with a NUL).
static PLANE_LABELS: Mutex<Vec<(usize, f32, f32, f32, Vec<u8>)>> = Mutex::new(Vec::new());
/// One button per card and plane type (`plane_loadouts::MAX_CRAFTS` types): index = craft * 9 + (card slot - 1); the click handler splits the number again.
const LOADOUT_NAMES: [&[u8]; 54] = [
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
    b"PLANE_ARM_19\0",
    b"PLANE_ARM_20\0",
    b"PLANE_ARM_21\0",
    b"PLANE_ARM_22\0",
    b"PLANE_ARM_23\0",
    b"PLANE_ARM_24\0",
    b"PLANE_ARM_25\0",
    b"PLANE_ARM_26\0",
    b"PLANE_ARM_27\0",
    b"PLANE_ARM_28\0",
    b"PLANE_ARM_29\0",
    b"PLANE_ARM_30\0",
    b"PLANE_ARM_31\0",
    b"PLANE_ARM_32\0",
    b"PLANE_ARM_33\0",
    b"PLANE_ARM_34\0",
    b"PLANE_ARM_35\0",
    b"PLANE_ARM_36\0",
    b"PLANE_ARM_37\0",
    b"PLANE_ARM_38\0",
    b"PLANE_ARM_39\0",
    b"PLANE_ARM_40\0",
    b"PLANE_ARM_41\0",
    b"PLANE_ARM_42\0",
    b"PLANE_ARM_43\0",
    b"PLANE_ARM_44\0",
    b"PLANE_ARM_45\0",
    b"PLANE_ARM_46\0",
    b"PLANE_ARM_47\0",
    b"PLANE_ARM_48\0",
    b"PLANE_ARM_49\0",
    b"PLANE_ARM_50\0",
    b"PLANE_ARM_51\0",
    b"PLANE_ARM_52\0",
    b"PLANE_ARM_53\0",
    b"PLANE_ARM_54\0",
];
static LOADOUT_GUIS: [std::sync::atomic::AtomicUsize; 54] = [const { std::sync::atomic::AtomicUsize::new(0) }; 54];
/// The ship each card slot's plane buttons were painted for. While cards move, a column follows the card that
/// holds this ship, not the slot number it is named after.
static PAINTED_PLANE_SHIPS: [std::sync::atomic::AtomicUsize; 9] = [const { std::sync::atomic::AtomicUsize::new(0) }; 9];
/// Gap (GUI units) between the ALLY button and the plane button above it.
const LOADOUT_GAP: f32 = 3.0;
/// The plane buttons are smaller than the ALLY button so both rows stay in the part of the screen that takes clicks.
const LOADOUT_SIZE: f32 = 44.0;
const LABEL_FONT: &std::ffi::CStr = c"flash_large_shadow";
static TOGGLE_GUIS: [std::sync::atomic::AtomicUsize; 8] = [const { std::sync::atomic::AtomicUsize::new(0) }; 8];
/// Identifies each builder visit for diagnostics and resets the bounded health-repair budget.
/// Scene teardown may destroy our elements; retained addresses are never lifetime guarantees.
static GUI_BUILD_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static GUI_SERVICED_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static GUI_HEALTH_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static GUI_LAST_SCAN_WARN_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static GUI_RECOVERY_TRIES: AtomicU32 = AtomicU32::new(0);
static GUI_RECOVERY_EXHAUSTED: AtomicBool = AtomicBool::new(false);
/// How often the unchanged-layout health check runs, and how many repairs it may make before it stays quiet.
const GUI_HEALTH_INTERVAL_MS: u64 = 1000;
const GUI_HEALTH_TRIES: u32 = 8;
/// Minimum gap (ms) between accepted clicks on one pre-combat button. The game can deliver one physical
/// click twice (press and release); a deliberate second click on the same button is not this fast.
const CLICK_DEBOUNCE_MS: u64 = 120;
/// Last accepted click per `PLANE_ARM_*` button, indexed by the number in the name.
static LAST_PLANE_CLICK_MS: [AtomicU64; 55] = [const { AtomicU64::new(0) }; 55];
/// Last accepted click per `ALLY_TOGGLE_*` button, indexed by the slot in the name.
static LAST_ALLY_CLICK_MS: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];
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
    // Every build (first entry, ship swap, RETRY) must revalidate retained addresses.
    GUI_BUILD_EPOCH.fetch_add(1, Ordering::Relaxed);
    precombat_dump("B2 0x1402d73a0 end");
}

type NewFn = unsafe extern "C" fn(usize) -> *mut u8;
type CtorFn = unsafe extern "C" fn(*mut u8, i32) -> *mut u8;
type GuiInitFn = unsafe extern "C" fn(*mut u8, *const u8, f32, f32, f32, f32, *const u8, *const u8, *const u8, *const u8, *const u8, f32);
type NodeAddFn = unsafe extern "C" fn(*mut u8, *mut u8, usize, usize);
type SetBgFn = unsafe extern "C" fn(*mut u8, i32, *const u8, f32, u8, i32) -> *mut u8;
type SetSpriteNameFn = unsafe extern "C" fn(*mut u8, *const u8, u8);
type ClearSpritesFn = unsafe extern "C" fn(*mut u8);
type ResetFn = unsafe extern "C" fn(*mut u8);
type SetPosFn = unsafe extern "C" fn(*mut u8, *const f32);
type FindFn = unsafe extern "C" fn(*const u8) -> usize;

fn toggle_line(message: String) {
    if TOGGLE_LINES.fetch_add(1, Ordering::Relaxed) < 3000 {
        log::info!("{message}");
    }
}

/// Swaps the sprite of a toggle to show its slot's choice.
unsafe fn paint_toggle(gui: *mut u8, slot: usize) -> bool {
    let on = SLOT_MASK.load(Ordering::Relaxed) & (1 << slot) != 0;
    paint_button(gui as usize, if on { SPRITE_ON } else { SPRITE_OFF })
}

/// Walks the game's GUI element list to its sentinel, including inactive entries.
/// The count-derived bound accommodates large screens without treating a truncated list as complete.
struct GuiScan {
    elements: Vec<usize>,
    /// False when the walk hit the cap before the sentinel: the list is only partially known.
    complete: bool,
    visited: usize,
    limit: usize,
}

unsafe fn scan_gui_list() -> GuiScan {
    let head: usize = read(GUI_LIST);
    let count: usize = read(GUI_COUNT);
    let limit = count.saturating_add(GUI_SCAN_SLACK).clamp(GUI_SCAN_MIN, GUI_SCAN_MAX);
    let mut elements = Vec::new();
    let mut visited = 0;
    let mut complete = false;
    if plausible(head) {
        let mut node: usize = read(head);
        for _ in 0..limit {
            if node == head {
                break;
            }
            if !plausible(node) {
                break;
            }
            visited += 1;
            let element: usize = read(node + 0x10);
            if !plausible(element) {
                break;
            }
            elements.push(element);
            node = read(node);
        }
        complete = node == head;
    }
    GuiScan {
        elements,
        complete,
        visited,
        limit,
    }
}

/// Writes the game's shown (2) / unused (0) state to one of our elements.
unsafe fn set_gui_active(gui: usize, active: bool) {
    if gui == 0 {
        return;
    }
    (gui as *mut i32).byte_add(OFF_ACTIVE).write_unaligned(if active { 2 } else { 0 });
}

/// Parent node of an element (`Node.master` at 0x80, its `Link.target` at +8 => 0x88 in 1.163). A live element
/// placed by `make_gui` is a child of the global root.
unsafe fn master_node(gui: usize) -> usize {
    read(gui + offset_of!(Node, master) + offset_of!(Link, target))
}

/// Cell of the `GUI_ELEMENTS` registry entry (+0xba8, see docs/ally-gui-v0.14.5-tester.md). Non-zero means the element is
/// registered and can still be found by name; it says nothing about the dead-node queue.
unsafe fn registry_cell(gui: usize) -> usize {
    read(gui + 0xba8)
}

/// Checks registry/root fields of an element already obtained from the current native list.
/// Callers must prove current membership BEFORE calling this function: it dereferences the pointer and is
/// not itself a lifetime check for a cached address.
unsafe fn gui_live_in_root(gui: usize) -> bool {
    if !plausible(gui) || registry_cell(gui) == 0 {
        return false;
    }
    let root: usize = read(GUI_ROOT);
    plausible(root) && master_node(gui) == root
}

/// Checks a GUI already obtained from the current complete `GUI_ELEMENTS` walk for the expected name,
/// registration and root ownership. Callers must establish membership before calling, especially for cached
/// addresses. Detached or foreign-parented objects are
/// never resurrected: `Node::add` does not clear a dead-node queue entry, so re-adding one could crash the
/// native `CollectDeadNodes` pass.
unsafe fn gui_reusable(gui: usize, name: &[u8]) -> bool {
    if !plausible(gui) {
        return false;
    }
    if cstr(gui + GUI_NAME).as_bytes() != &name[..name.len() - 1] {
        return false;
    }
    // only a clean hidden (0) or active (2) object may be restored; 1 = removed/dying
    match read::<i32>(gui + OFF_ACTIVE) {
        0 | 2 => {}
        _ => return false,
    }
    gui_live_in_root(gui)
}

/// Calls the element's vtable slot +0x88 (`GUI::Load`): resolves the element's own media names and loads the
/// resources of every sprite in its list, then recurses into children. Only call on a newly constructed GUI
/// or a current registry member validated by `get_or_make`, on the game thread outside rendering.
unsafe fn load_gui(gui: usize) {
    if gui == 0 {
        return;
    }
    let vtable: usize = read(gui);
    if !plausible(vtable) {
        return;
    }
    let load: ResetFn = std::mem::transmute(read::<usize>(vtable + 0x88));
    load(gui as *mut u8);
}

/// begin/end of the element's sprite vector (`Body.sprites`, 0x8c0), when the vector is sane.
unsafe fn sprite_bounds(gui: usize) -> Option<(usize, usize)> {
    let begin: usize = read(gui + offset_of!(Body, sprites));
    let end: usize = read(gui + offset_of!(Body, sprites) + 8);
    let capacity: usize = read(gui + offset_of!(Body, sprites) + 16);
    // CVec's three pointers are private upstream; read their scalar native header, never an owning CVec.
    if begin == 0 && end == 0 && capacity == 0 {
        return Some((0, 0));
    }
    if !plausible(begin) || end < begin || capacity < end {
        return None;
    }

    let used_bytes = end - begin;
    let available_bytes = capacity - begin;

    if !used_bytes.is_multiple_of(GUI_SPRITE_SLOT_SIZE)
        || !available_bytes.is_multiple_of(GUI_SPRITE_SLOT_SIZE)
    {
        return None;
    }

    let sprite_capacity = available_bytes / GUI_SPRITE_SLOT_SIZE;
    if sprite_capacity > GUI_SPRITE_LIMIT {
        return None;
    }
    Some((begin, end))
}

/// The newest Sprite of one of our elements, when the vector is sane and non-empty.
unsafe fn newest_sprite(gui: usize) -> Option<usize> {
    let (begin, end) = sprite_bounds(gui)?;
    if end == begin {
        return None;
    }
    let sprite: usize = read(end - 8);
    plausible(sprite).then_some(sprite)
}

/// 0 = no sprite list entry, 1 = newest sprite exists but its resource is not loaded, 2 = newest sprite loaded.
/// The sprite vector is `Body::sprites` (0x8c0, highfleet-rs layout). A Sprite keeps its loaded HGE
/// animation/media handle at +0x78 (`SpriteLoad_` 0x1402dfef0; it stays null when the name cannot be loaded).
unsafe fn gui_sprite_state(gui: usize) -> u32 {
    let Some(sprite) = newest_sprite(gui) else { return 0 };
    if read::<usize>(sprite + 0x78) == 0 {
        1
    } else {
        2
    }
}

/// Maintains exactly one native-owned button sprite. The native setter is already a no-op for an unchanged
/// resource name, so no parallel Rust sprite-state cache is needed.
///
/// SAFETY: callers provide a newly constructed or current-list-validated custom GUI on the game thread,
/// outside rendering. Its bounded native sprite vector owns the Sprite objects; native functions retain
/// allocation/destruction and string ownership. No owning Rust copies or references to game memory are made.
/// Native contracts: docs/ally-gui-v0.14.5-tester.md, verified Steam 1.163 routines listed there.
unsafe fn paint_button(gui: usize, name: &[u8]) -> bool {
    let Some((begin, end)) = sprite_bounds(gui) else {
        toggle_line("ALLY GUI paint refused: invalid native sprite vector".to_string());
        return false;
    };
    let count = (end - begin) / 8;
    if count == 1 {
        let sprite: usize = read(begin);
        if !plausible(sprite) {
            toggle_line("ALLY GUI paint refused: invalid native sprite pointer".to_string());
            return false;
        }
        let set_name: SetSpriteNameFn = std::mem::transmute(SET_SPRITE_NAME);
        set_name(sprite as *mut u8, name.as_ptr(), 0);
    } else {
        if count > 1 {
            // Validate the complete bounded pointer array before the native destructor walk.
            for address in (begin..end).step_by(8) {
                let sprite: usize = read(address);
                if !plausible(sprite) || !plausible(read::<usize>(sprite)) {
                    toggle_line("ALLY GUI paint refused: invalid native sprite entry".to_string());
                    return false;
                }
            }
            let clear: ClearSpritesFn = std::mem::transmute(OBJECT_ARRAY_DELETE_ALL);
            clear(gui as *mut u8);
        }
        let set_bg: SetBgFn = std::mem::transmute(GUI_SETBG);
        let sprite = set_bg(gui as *mut u8, 0, name.as_ptr(), read::<f32>(BG_MINUS_ONE), 0, -1);
        if sprite.is_null() {
            toggle_line("ALLY GUI paint failed: native sprite creation returned null".to_string());
            return false;
        }
    }
    load_gui(gui);
    true
}

/// Counts for the per-build diagnostic line. Only pointers present in the current complete list are dereferenced.
unsafe fn owned_sprite_summary(listed: &[usize]) -> (u32, u32, u32, u32, u32) {
    let root: usize = read(GUI_ROOT);
    let (mut active, mut unloaded, mut detached, mut invisible, mut abandoned) = (0, 0, 0, 0, 0);
    for kept in TOGGLE_GUIS.iter().chain(LOADOUT_GUIS.iter()) {
        let gui = kept.load(Ordering::Relaxed);
        if gui == 0 {
            continue;
        }
        if !listed.contains(&gui) {
            abandoned += 1;
            continue;
        }
        if !plausible(gui) {
            continue;
        }
        if read::<i32>(gui + OFF_ACTIVE) == 2 {
            active += 1;
            if gui_sprite_state(gui) < 2 {
                unloaded += 1;
            }
        }
        if master_node(gui) != root {
            detached += 1;
        }
        if read::<f32>(gui + offset_of!(Body, opacity)) <= 0.0 {
            invisible += 1;
        }
    }
    (active, unloaded, detached, invisible, abandoned)
}

/// Rate-limited note that a list walk was incomplete, so a truncated scan is never mistaken for "no cards".
unsafe fn warn_incomplete_scan(visited: usize, limit: usize) {
    let now = now_ms();
    if now.wrapping_sub(GUI_LAST_SCAN_WARN_MS.load(Ordering::Relaxed)) < 2000 {
        return;
    }
    GUI_LAST_SCAN_WARN_MS.store(now, Ordering::Relaxed);
    toggle_line(format!(
        "ALLY GUI scan incomplete: visited {visited} of at most {limit} list nodes (GUI count {}); screen state not inferred",
        read::<usize>(GUI_COUNT)
    ));
}

/// The player's ship cards on the pre-combat screen (`PRECOMBAT_A`; `PRECOMBAT_B` are the enemy's), left to right:
/// (x, y, width, height). Positions are the float pairs at +0x128, sizes at +0x218.
struct CardScan {
    cards: Vec<(f32, f32, f32, f32)>,
    elements: Vec<usize>,
    complete: bool,
    visited: usize,
    limit: usize,
}

unsafe fn scan_cards(quiet: bool) -> CardScan {
    let scan = scan_gui_list();
    let mut cards = Vec::new();
    let mut other = Vec::new();
    for &element in &scan.elements {
        if read::<i32>(element + OFF_ACTIVE) == 0 {
            continue;
        }
        let name = cstr(element + GUI_NAME);
        let entry = (
            read::<f32>(element + offset_of!(Body, position_x)),
            read::<f32>(element + offset_of!(Body, position_y)),
            read::<f32>(element + 0x218),
            read::<f32>(element + 0x21c),
        );
        if name == "PRECOMBAT_A" {
            cards.push(entry);
        } else if !quiet && (name == "PRECOMBAT_B" || name == "MINIFIG_NUMBER" || name == "MES_APP") {
            other.push(format!("{name} ({:.0},{:.0} {:.0}x{:.0})", entry.0, entry.1, entry.2, entry.3));
        }
    }
    cards.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    if !quiet {
        toggle_line(format!(
            "ALLY cards: {} ship cards {:?} | others {} | scan {}/{} complete {}",
            cards.len(),
            cards.iter().map(|c| format!("({:.0},{:.0} {:.0}x{:.0})", c.0, c.1, c.2, c.3)).collect::<Vec<_>>(),
            other.join(" "),
            scan.visited,
            scan.limit,
            scan.complete,
        ));
    }
    CardScan {
        cards,
        elements: scan.elements,
        complete: scan.complete,
        visited: scan.visited,
        limit: scan.limit,
    }
}

/// Builds one GUI element the way the game builds PRECOMBAT_OK (sequence read from `0x1402d7620`).
unsafe fn make_gui(name: &[u8]) -> *mut u8 {
    let root: usize = read(GUI_ROOT);
    if root == 0 {
        return std::ptr::null_mut();
    }
    let new: NewFn = std::mem::transmute(GUI_NEW);
    let ctor: CtorFn = std::mem::transmute(GUI_CTOR);
    let mem = new(size_of::<Gui>());
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

/// Gets the element called `name`: a live one the game still lists, else our kept (hidden) one, else a new one.
/// Returns (gui, created).
///
/// A kept pointer is dereferenced only after the current complete `GUI_ELEMENTS` walk lists it, and it is reused
/// only while it is registered and still a child of the global root. A detached/foreign-parented object may be
/// queued for native collection; it is never re-added or resurrected, just abandoned and replaced.
///
/// The same rule guards every branch: a cached pointer can outlive its element after a screen rebuild (observed
/// as a read at old `ALLY_TOGGLE_2 + 0xa40` in a B2 crash log), so no name/state/registry read is made before
/// membership in the current list is established.
unsafe fn get_or_make(
    name: &[u8],
    kept_slot: &std::sync::atomic::AtomicUsize,
    listed: &[usize],
    complete: bool,
) -> Option<(usize, bool)> {
    if !complete {
        // an incomplete list cannot prove the kept object is gone; do not create duplicates
        return None;
    }
    let kept = kept_slot.load(Ordering::Relaxed);
    if listed.contains(&kept) && gui_reusable(kept, name) {
        set_gui_active(kept, true);
        return Some((kept, false));
    }
    // The snapshot also includes hidden objects. Never fall back to an unbounded native search after a
    // truncated scan, and never read an unlisted cached pointer merely to compare its old name.
    for &gui in listed {
        if gui != kept && gui_reusable(gui, name) {
            set_gui_active(gui, true);
            kept_slot.store(gui, Ordering::Relaxed);
            return Some((gui, false));
        }
    }
    if kept != 0 {
        toggle_line(format!(
            "ALLY GUI: retained {} is no longer reusable; discarding its cached address without reviving it",
            String::from_utf8_lossy(&name[..name.len() - 1])
        ));
        // drop the cached number before allocating: a stale pointer must never be read again
        kept_slot.store(0, Ordering::Relaxed);
    }
    let ptr = make_gui(name);
    if ptr.is_null() {
        return None;
    }
    kept_slot.store(ptr as usize, Ordering::Relaxed);
    Some((ptr as usize, true))
}

/// The ship creature of each player card (`PRECOMBAT_A`), left to right: the card's `+0xbc0` is a Property pointer, `GetPropertyLinkedObject` gives the ship.
unsafe fn card_ship_list() -> Vec<(f32, usize)> {
    let scan = scan_gui_list();
    if !scan.complete {
        return Vec::new();
    }
    card_ships_from(&scan.elements)
}

/// Reads ship links from the same complete snapshot as the card layout, so a second incomplete walk cannot
/// turn existing carriers into apparent empty slots during placement or health checks.
unsafe fn card_ships_from(listed: &[usize]) -> Vec<(f32, usize)> {
    let mut out = Vec::new();
    for &element in listed {
        if read::<i32>(element + OFF_ACTIVE) != 0 && cstr(element + GUI_NAME) == "PRECOMBAT_A" {
            out.push((read::<f32>(element + offset_of!(Body, position_x)), plane_loadouts::ship_of_link(read::<usize>(element + GUI_LINK_TARGET))));
        }
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

/// Moves the text beside a plane button; the text itself is kept.
fn move_plane_label(slot: usize, x: f32, y: f32, size: f32) {
    if let Ok(mut labels) = PLANE_LABELS.lock() {
        if let Some(label) = labels.iter_mut().find(|label| label.0 == slot) {
            label.1 = x;
            label.2 = y;
            label.3 = size;
        }
    }
}

/// Paints the plane button with the sprite of its ship's current choice.
unsafe fn paint_loadout(gui: *mut u8, ship: usize, craft: usize) -> bool {
    let sprite = if plane_loadouts::choice(ship, craft).is_some() { plane_loadouts::SPRITE_CHOSEN } else { plane_loadouts::SPRITE_DEFAULT };
    paint_button(gui as usize, sprite)
}

/// The ship carries plane type `craft` and the player can still choose an armament for it (or already did): it gets a button.
unsafe fn loadout_wanted(ship: usize, craft: usize) -> bool {
    plane_loadouts::has_craft(ship, craft) && (!plane_loadouts::available_arms(ship, craft).is_empty() || plane_loadouts::choice(ship, craft).is_some())
}

/// Places, shows or hides the button of plane type `craft` of card `card` (1-based); true when it is shown.
unsafe fn place_loadout(
    card: usize,
    craft: usize,
    ship: Option<usize>,
    pos: [f32; 2],
    size: f32,
    listed: &[usize],
    complete: bool,
) -> bool {
    let slot = craft * 9 + card; // 1-based button number
    let kept_slot = &LOADOUT_GUIS[slot - 1];
    let name = LOADOUT_NAMES[slot - 1];
    let shown = ship.filter(|&s| loadout_wanted(s, craft));
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
        if listed.contains(&kept)
            && cstr(kept + GUI_NAME).as_bytes() == &name[..name.len() - 1]
            && read::<i32>(kept + OFF_ACTIVE) == 2
        {
            set_gui_active(kept, false);
        }
        return false;
    };
    let previous = kept_slot.load(Ordering::Relaxed);
    let Some((gui, created)) = get_or_make(name, kept_slot, listed, complete) else {
        toggle_line(format!("PLANE button {slot}: not created"));
        return false;
    };
    let ptr = gui as *mut u8;
    // a button the battle start switched off may still be found in the game's list (a RETRY in Shipworks): switch it on again
    set_gui_active(gui, true);
    if !paint_loadout(ptr, ship, craft) {
        set_gui_active(gui, false);
        set_plane_label(slot, None);
        return false;
    }
    if created || gui != previous {
        if created {
            let reset: ResetFn = std::mem::transmute(GUI_RESET);
            reset(ptr);
        }
        kept_slot.store(gui, Ordering::Relaxed);
    }
    move_gui(gui, pos, size);
    // a re-shown button must reload its sprite resources after a screen rebuild
    load_gui(gui);
    set_plane_label(slot, Some((pos[0], pos[1], size, plane_loadouts::label_for(craft, plane_loadouts::choice(ship, craft)))));
    toggle_line(format!("PLANE button {slot}: ship {ship:#x} {} at ({:.0},{:.0}) size {size:.0}, choice {:?}", plane_loadouts::crafts()[craft].label, pos[0], pos[1], plane_loadouts::choice(ship, craft).map(|a| plane_loadouts::arms()[a].key.clone())));
    true
}

/// A signature of the card positions alone (no ship links): a change here means the cards are moving.
fn layout_signature(cards: &[(f32, f32, f32, f32)]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ cards.len() as u64;
    for c in cards {
        h = (h ^ c.0.to_bits() as u64).wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// A signature of the card layout and the ships behind the cards: two cards swapping places keep the same x
/// positions, so the ship links count too when the plane buttons (which belong to the ship) are shown.
fn card_signature(cards: &[(f32, f32, f32, f32)], ships: &[(f32, usize)]) -> u64 {
    let mut h = layout_signature(cards);
    if PLANE_BUTTONS.load(Ordering::Relaxed) {
        for (_, ship) in ships {
            h = (h ^ *ship as u64).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

/// Cards that all report one spot have not been laid out yet. Until they are, the grid read off the screenshots
/// stands in, so the buttons are not stacked in the middle of the screen.
fn cards_stacked(cards: &[(f32, f32, f32, f32)]) -> bool {
    cards.len() > 1 && cards.iter().all(|c| (c.0 - cards[0].0).abs() < 1.0)
}

fn grid_cards(cards: &[(f32, f32, f32, f32)]) -> Vec<(f32, f32, f32, f32)> {
    cards
        .iter()
        .enumerate()
        .map(|(i, c)| (GRID_X0 + GRID_STEP * i as f32, GRID_Y - TOGGLE_DY, c.2, c.3))
        .collect()
}

/// Where a button of `slot` (1-based) goes: the card's x, its offset to the right and its size. The gap to the
/// next card decides the offset and size, so a button still fits between two ship numbers when cards are close.
fn slot_geometry(cards: &[(f32, f32, f32, f32)], slot: usize) -> Option<(f32, f32, f32)> {
    let &(x, _, _, _) = cards.get(slot - 1)?;
    let gap = cards.get(slot).map(|next| next.0 - x);
    let dx = gap.map_or(TOGGLE_DX, |gap| TOGGLE_DX.min(gap * 0.5));
    let size = gap.map_or(TOGGLE_SIZE, |gap| (gap - NUMBER_WIDTH).clamp(TOGGLE_MIN_SIZE, TOGGLE_SIZE));
    Some((x, dx, size))
}

/// Moves an already placed element and resizes its hit box. The sprite is drawn from the element's position every
/// frame, so no resource reload is needed while following moving cards.
unsafe fn move_gui(gui: usize, pos: [f32; 2], size: f32) {
    let set_pos: SetPosFn = std::mem::transmute(GUI_SETPOS);
    set_pos(gui as *mut u8, pos.as_ptr());
    (gui as *mut f32).byte_add(0x218).write_unaligned(size);
    (gui as *mut f32).byte_add(0x21c).write_unaligned(size);
}

/// Places (creating on first use) the ALLY buttons and the slot-1 marker beside the ship numbers. Slot n is the
/// n-th card from the left (`precombatOk` orders the squad by card x), so after the player shuffles the cards
/// this is called again and every button goes to the card now standing in its slot.
unsafe fn ensure_toggles() {
    let scan = scan_cards(false);
    if !scan.complete {
        warn_incomplete_scan(scan.visited, scan.limit);
        return;
    }
    let mut cards = scan.cards;
    let fresh_visit = !PRECOMBAT_LIVE.load(Ordering::Relaxed);
    let build_epoch = GUI_BUILD_EPOCH.load(Ordering::Relaxed);
    let new_build = build_epoch != GUI_SERVICED_EPOCH.load(Ordering::Relaxed);
    if fresh_visit || new_build {
        GUI_RECOVERY_TRIES.store(0, Ordering::Relaxed);
        GUI_RECOVERY_EXHAUSTED.store(false, Ordering::Relaxed);
        // give the GUI pass time to register freshly created children before the first health check
        GUI_HEALTH_MS.store(now_ms(), Ordering::Relaxed);
    }
    if fresh_visit {
        // a fresh visit starts with no armament chosen; clear before painting so no stale icon or label is shown
        plane_loadouts::clear();
    }
    if new_build {
        toggle_line(format!(
            "ALLY pre-combat screen build {build_epoch}: {} cards, scan {}/{} complete {}",
            cards.len(),
            scan.visited,
            scan.limit,
            scan.complete
        ));
    }
    // cards stacked on one spot (not laid out yet): fall back to a grid read off the screenshots
    if cards_stacked(&cards) {
        cards = grid_cards(&cards);
        toggle_line("ALLY cards report no layout: using the screenshot grid".to_string());
    }
    let ships = if PLANE_BUTTONS.load(Ordering::Relaxed) { card_ships_from(&scan.elements) } else { Vec::new() };
    let signature = card_signature(&cards, &ships);
    let mut layout = Vec::new();
    // slot 1 (the flagship, always the player's ship) gets the marker, slots 2.. the toggles
    for slot in 1..=9usize {
        let Some((x, dx, size)) = slot_geometry(&cards, slot) else {
            if slot == 1 {
                if scan.complete {
                    MARKER_SHOW.store(false, Ordering::Relaxed);
                }
                continue;
            }
            // no such ship on this screen: hide a button kept from an earlier build. An incomplete scan does
            // not prove the slot is absent, so nothing is hidden from a partial list.
            if scan.complete {
                let kept = TOGGLE_GUIS[slot - 2].load(Ordering::Relaxed);
                if scan.elements.contains(&kept)
                    && cstr(kept + GUI_NAME) == format!("ALLY_TOGGLE_{slot}")
                    && read::<i32>(kept + OFF_ACTIVE) == 2
                {
                    set_gui_active(kept, false);
                }
            }
            continue;
        };
        if slot == 1 {
            // the slot-1 label is text drawn from a render seam (`draw_marker`), not a GUI element: it only needs the spot
            MARKER_X.store((x + dx + MARKER_DX).to_bits(), Ordering::Relaxed);
            MARKER_SHOW.store(true, Ordering::Relaxed);
            layout.push(format!("1@{x:.0}+{dx:.0}/text"));
            continue;
        }
        let (name, kept_slot) = (TOGGLE_NAMES[slot - 2], &TOGGLE_GUIS[slot - 2]);
        let Some((gui, created)) = get_or_make(name, kept_slot, &scan.elements, scan.complete) else {
            toggle_line(format!("ALLY toggle {slot}: not created"));
            continue;
        };
        let ptr = gui as *mut u8;
        set_gui_active(gui, true);
        if !paint_toggle(ptr, slot) {
            set_gui_active(gui, false);
            continue;
        }
        if created {
            let reset: ResetFn = std::mem::transmute(GUI_RESET);
            reset(ptr);
        }
        kept_slot.store(gui, Ordering::Relaxed);
        // the numbers sit at one height for every card, so the buttons do too (the card height varies per ship class)
        move_gui(gui, [x + dx, GRID_Y], size);
        // Load after the move: reloads resources after a screen rebuild
        load_gui(gui);
        layout.push(format!("{slot}@{:.0}+{:.0}/{:.0}", x, dx, size));
    }
    if PLANE_BUTTONS.load(Ordering::Relaxed) {
        // Reuse still-live plane buttons without abandoning hidden elements. Scene teardown may also have
        // freed them, in which case get_or_make discards the stale address before creating a replacement.
        let mut shown: Vec<usize> = Vec::new();
        for slot in 1..=9usize {
            let Some((x, dx, size)) = slot_geometry(&cards, slot) else {
                if scan.complete {
                    for craft in 0..plane_loadouts::crafts().len() {
                        place_loadout(slot, craft, None, [0.0, 0.0], 0.0, &scan.elements, scan.complete);
                    }
                }
                PAINTED_PLANE_SHIPS[slot - 1].store(0, Ordering::Relaxed);
                continue;
            };
            // plane buttons per card: ONE column right of the ALLY button, one row per plane type the ship carries (in the
            // game's craft order), the first row level with the ALLY button and the rest below it, each label to its button's
            // right so no two labels meet. The space under the column is free (user, 2026-10-25), so the buttons keep their
            // size: six rows of 64 units fit between the ALLY row and the bottom edge of the panel.
            let ship = ships.get(slot - 1).map(|s| s.1);
            let psize = size;
            let mut row = 0usize;
            for craft in 0..plane_loadouts::crafts().len() {
                let pos = [x + dx + size + LOADOUT_GAP * 2.0, GRID_Y + row as f32 * (psize + LOADOUT_GAP)];
                if place_loadout(slot, craft, ship, pos, psize, &scan.elements, scan.complete) {
                    shown.push(craft * 9 + slot);
                    row += 1;
                }
            }
            PAINTED_PLANE_SHIPS[slot - 1].store(ship.unwrap_or(0), Ordering::Relaxed);
        }
        if scan.complete {
            hide_plane_buttons(&shown);
        }
    }
    PLACED_SIG.store(signature, Ordering::Relaxed);
    PLACED_LAYOUT_SIG.store(layout_signature(&cards), Ordering::Relaxed);
    if fresh_visit {
        ENTRY_MS.store(now_ms(), Ordering::Relaxed); // a fresh entry: the roller covers are about to play
        ROLL_SEEN.store(false, Ordering::Relaxed);
    }
    PRECOMBAT_LIVE.store(true, Ordering::Relaxed);
    GUI_SERVICED_EPOCH.store(build_epoch, Ordering::Relaxed);
    if new_build {
        let final_scan = scan_gui_list();
        if final_scan.complete {
            let (active, unloaded, detached, invisible, abandoned) = owned_sprite_summary(&final_scan.elements);
            toggle_line(format!(
                "ALLY GUI build {build_epoch}: {active} owned elements active, {unloaded} without a loaded sprite, {detached} detached, {invisible} opacity 0, {abandoned} retained but unlisted"
            ));
        }
    }
    toggle_line(format!("ALLY placed (slot@card x+dx/size): {}", layout.join(" ")));
}

/// Moves the plane buttons of `slot` that are shown, and their labels. Rows are counted in the same order as the
/// placement, so a missing type does not collapse the rows into each other while the cards move.
unsafe fn follow_plane_buttons(listed: &[usize], slot: usize, x: f32, dx: f32, size: f32) {
    let mut row = 0usize;
    for craft in 0..plane_loadouts::crafts().len() {
        let number = craft * 9 + slot;
        let gui = LOADOUT_GUIS[number - 1].load(Ordering::Relaxed);
        if !listed.contains(&gui) || !gui_reusable(gui, LOADOUT_NAMES[number - 1]) || read::<i32>(gui + OFF_ACTIVE) != 2 {
            continue;
        }
        let pos = [x + dx + size + LOADOUT_GAP * 2.0, GRID_Y + row as f32 * (size + LOADOUT_GAP)];
        move_gui(gui, pos, size);
        move_plane_label(number, pos[0], pos[1], size);
        row += 1;
    }
}

/// Moves each plane column with the card that holds the ship it was painted for. The column keeps the slot-based
/// button name it was built with; the full placement re-keys it to the new slot after the cards rest. Following
/// the ship (not the slot) keeps the column attached to the carrier while two cards cross.
unsafe fn follow_plane_columns(cards: &[(f32, f32, f32, f32)], listed: &[usize], ships: &[(f32, usize)]) {
    if !PLANE_BUTTONS.load(Ordering::Relaxed) {
        return;
    }
    for (index, &(_, ship)) in ships.iter().enumerate() {
        if ship == 0 {
            continue;
        }
        let Some(painted_slot) = (1..=9usize).find(|&slot| PAINTED_PLANE_SHIPS[slot - 1].load(Ordering::Relaxed) == ship) else {
            continue;
        };
        let Some((x, dx, size)) = slot_geometry(cards, index + 1) else {
            continue;
        };
        follow_plane_buttons(listed, painted_slot, x, dx, size);
    }
}

/// Moves the buttons that already exist to where the cards are now. Runs on every frame while the cards move
/// (dragged or animating to their new slots), so the buttons travel with them instead of jumping into place after
/// the cards settle. Does not create, paint or log; false means a button is missing and a full placement is needed.
unsafe fn follow_toggles(cards: &[(f32, f32, f32, f32)], listed: &[usize], ships: &[(f32, usize)], layout: u64) -> bool {
    follow_plane_columns(cards, listed, ships);
    let mut complete = true;
    for slot in 1..=9usize {
        let Some((x, dx, size)) = slot_geometry(cards, slot) else {
            break;
        };
        if slot == 1 {
            MARKER_X.store((x + dx + MARKER_DX).to_bits(), Ordering::Relaxed);
            continue;
        }
        let name = TOGGLE_NAMES[slot - 2];
        let gui = TOGGLE_GUIS[slot - 2].load(Ordering::Relaxed);
        if !listed.contains(&gui) || !gui_reusable(gui, name) || read::<i32>(gui + OFF_ACTIVE) != 2 {
            complete = false;
            continue;
        }
        move_gui(gui, [x + dx, GRID_Y], size);
    }
    PLACED_LAYOUT_SIG.store(layout, Ordering::Relaxed);
    complete
}

/// Per-frame check while the pre-combat screen is up (called from the game thread by two hooks): the buttons
/// follow moving cards immediately; a full placement (repaint) runs once the cards have rested.
/// How long (ms) the player's cards may be missing before the pre-combat screen counts as gone (a ship swap rebuilds them).
const SCREEN_GONE_MS: u64 = 1500;
static CARDS_MISSING_SINCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// True (after hiding our elements) when the pre-combat screen has been torn down without a click we saw (Esc then quit,
/// a save load, a scene change): `PRECOMBAT_LIVE` would stay set and our text would be drawn over the city view.
unsafe fn screen_gone() -> bool {
    if !PRECOMBAT_LIVE.load(Ordering::Relaxed) {
        return false;
    }
    let scan = scan_cards(true);
    if !scan.complete {
        warn_incomplete_scan(scan.visited, scan.limit);
        CARDS_MISSING_SINCE.store(0, Ordering::Relaxed);
        return false;
    }
    if !scan.cards.is_empty() {
        CARDS_MISSING_SINCE.store(0, Ordering::Relaxed);
        return false;
    }
    let now = now_ms();
    let since = CARDS_MISSING_SINCE.load(Ordering::Relaxed);
    if since == 0 {
        CARDS_MISSING_SINCE.store(now, Ordering::Relaxed);
        return false;
    }
    if now.wrapping_sub(since) < SCREEN_GONE_MS {
        return false;
    }
    CARDS_MISSING_SINCE.store(0, Ordering::Relaxed);
    hide_toggles("screen gone");
    true
}

/// The Esc menu's dimming shade (`QUIT_BLACK`, built by 0x1401ecb10) is a GUI element, drawn under our seam text: while
/// it exists the text would stay bright, so it is not drawn.
unsafe fn esc_menu_open() -> bool {
    let find: FindFn = std::mem::transmute(GUI_FIND);
    find(b"QUIT_BLACK\0".as_ptr()) != 0
}

/// Why one of our elements is not in its expected visible state. `None` = it is shown and its newest sprite is loaded.
/// The separate reasons keep an expected-disabled element distinguishable from a sprite whose media is not loaded.
unsafe fn gui_problem(gui: usize, name: &[u8], listed: &[usize]) -> Option<&'static str> {
    if !plausible(gui) {
        return Some("no usable pointer");
    }
    if !listed.contains(&gui) {
        return Some("not in the current GUI list (reuse refused)");
    }
    if registry_cell(gui) == 0 {
        return Some("registry cell 0 (reuse refused)");
    }
    if cstr(gui + GUI_NAME).as_bytes() != &name[..name.len() - 1] {
        return Some("stale pointer (name mismatch)");
    }
    let root: usize = read(GUI_ROOT);
    if !plausible(root) {
        return Some("GUI root unavailable");
    }
    let master = master_node(gui);
    if master != root {
        return Some(if master == 0 {
            "detached from the GUI root (reuse refused, not re-added)"
        } else {
            "foreign parent (reuse refused)"
        });
    }
    match read::<i32>(gui + OFF_ACTIVE) {
        0 => return Some("disabled (active flag 0)"),
        2 => {}
        _ => return Some("removed/dying or unexpected state (reuse refused)"),
    }
    match gui_sprite_state(gui) {
        0 => Some("no sprite attached"),
        1 => Some("sprite resource missing"),
        _ => None,
    }
}

/// Compares the elements we own with the cards that are on screen now. Does not change anything.
unsafe fn gui_health(scan: &CardScan, cards: &[(f32, f32, f32, f32)]) -> Vec<String> {
    let mut issues = Vec::new();
    if !scan.complete {
        issues.push(format!("card scan incomplete ({}/{})", scan.visited, scan.limit));
        return issues;
    }
    let expected_slots = cards.len().min(9);
    for slot in 2..=expected_slots {
        if let Some(why) = gui_problem(TOGGLE_GUIS[slot - 2].load(Ordering::Relaxed), TOGGLE_NAMES[slot - 2], &scan.elements) {
            issues.push(format!("ALLY_TOGGLE_{slot}: {why}"));
        }
    }
    // kept buttons for slots that no longer have a card must not stay visible
    for slot in (expected_slots.max(1) + 1)..=9 {
        let gui = TOGGLE_GUIS[slot - 2].load(Ordering::Relaxed);
        if scan.elements.contains(&gui)
            && cstr(gui + GUI_NAME) == format!("ALLY_TOGGLE_{slot}")
            && read::<i32>(gui + OFF_ACTIVE) == 2
        {
            issues.push(format!("ALLY_TOGGLE_{slot}: still visible without a card"));
        }
    }
    if PLANE_BUTTONS.load(Ordering::Relaxed) {
        let ships = card_ships_from(&scan.elements);
        for slot in 1..=9usize {
            let ship = if slot <= expected_slots { ships.get(slot - 1).map(|s| s.1) } else { None };
            for craft in 0..plane_loadouts::crafts().len() {
                let number = craft * 9 + slot;
                let gui = LOADOUT_GUIS[number - 1].load(Ordering::Relaxed);
                let wanted = ship.is_some_and(|s| loadout_wanted(s, craft));
                let problem = if wanted {
                    gui_problem(gui, LOADOUT_NAMES[number - 1], &scan.elements)
                } else if scan.elements.contains(&gui)
                    && cstr(gui + GUI_NAME).as_bytes() == &LOADOUT_NAMES[number - 1][..LOADOUT_NAMES[number - 1].len() - 1]
                    && read::<i32>(gui + OFF_ACTIVE) == 2
                {
                    Some("still visible without a plane")
                } else {
                    None
                };
                if let Some(why) = problem {
                    issues.push(format!("PLANE_ARM_{number}: {why}"));
                }
            }
        }
    }
    issues
}

/// Bounded repair while the card layout is unchanged: at most one pass per second and `GUI_HEALTH_TRIES`
/// consecutive repairs. A healthy check or new build resets the budget. Repairs reuse validated elements; a fresh one is created
/// only when reuse is refused, so the budget also bounds any replacement objects.
unsafe fn gui_health_tick(scan: &CardScan, cards: &[(f32, f32, f32, f32)], now: u64) {
    if now.wrapping_sub(GUI_HEALTH_MS.load(Ordering::Relaxed)) < GUI_HEALTH_INTERVAL_MS {
        return;
    }
    GUI_HEALTH_MS.store(now, Ordering::Relaxed);
    let issues = gui_health(scan, cards);
    if issues.is_empty() {
        GUI_RECOVERY_TRIES.store(0, Ordering::Relaxed);
        GUI_RECOVERY_EXHAUSTED.store(false, Ordering::Relaxed);
        return;
    }
    if !scan.complete || GUI_RECOVERY_EXHAUSTED.load(Ordering::Relaxed) {
        return;
    }
    let attempt = GUI_RECOVERY_TRIES.fetch_add(1, Ordering::Relaxed) + 1;
    let shown: Vec<&str> = issues.iter().take(6).map(String::as_str).collect();
    let more = issues.len().saturating_sub(shown.len());
    let summary = format!("{}{}", shown.join("; "), if more > 0 { format!(" (+{more} more)") } else { String::new() });
    if attempt > GUI_HEALTH_TRIES {
        GUI_RECOVERY_EXHAUSTED.store(true, Ordering::Relaxed);
        toggle_line(format!("ALLY GUI health: repair budget of {GUI_HEALTH_TRIES} used, staying quiet: {summary}"));
        return;
    }
    toggle_line(format!("ALLY GUI health repair {attempt}/{GUI_HEALTH_TRIES}: {summary}"));
    ensure_toggles();
}

unsafe fn precombat_tick(source: usize) {
    if screen_gone() {
        return;
    }
    if !PRECOMBAT_LIVE.load(Ordering::Relaxed) {
        return;
    }
    if TICKS_SEEN[source].fetch_add(1, Ordering::Relaxed) == 0 {
        toggle_line(format!("ALLY precombat tick: first call from hook {source} (0 = input task, 1 = battle manager)"));
    }
    let scan = scan_cards(true);
    if !scan.complete {
        // no placement, hiding or health decision from a partial list
        warn_incomplete_scan(scan.visited, scan.limit);
        return;
    }
    if scan.cards.is_empty() {
        return;
    }
    let grid;
    let cards: &[(f32, f32, f32, f32)] = if cards_stacked(&scan.cards) {
        grid = grid_cards(&scan.cards);
        &grid
    } else {
        &scan.cards
    };
    let layout = layout_signature(cards);
    let ships = if PLANE_BUTTONS.load(Ordering::Relaxed) { card_ships_from(&scan.elements) } else { Vec::new() };
    let sig = card_signature(cards, &ships);
    let now = now_ms();
    if layout != TICK_SIG.swap(layout, Ordering::Relaxed) {
        TICK_CHANGED_MS.store(now, Ordering::Relaxed);
        if diag::debug() && TICK_LOGS.fetch_add(1, Ordering::Relaxed) < 4000 {
            log::info!(
                "ALLY card layout moving: {:?}",
                cards.iter().map(|c| format!("{:.1}", c.0)).collect::<Vec<_>>()
            );
        }
    }
    if layout != PLACED_LAYOUT_SIG.load(Ordering::Relaxed) {
        // the cards are somewhere else than the buttons: move the buttons with them on this and every frame
        if !follow_toggles(cards, &scan.elements, &ships, layout) {
            ensure_toggles();
        }
        return;
    }
    if sig != PLACED_SIG.load(Ordering::Relaxed) && now.wrapping_sub(TICK_CHANGED_MS.load(Ordering::Relaxed)) >= SETTLE_MS {
        toggle_line("ALLY cards settled: placing the buttons again".to_string());
        ensure_toggles();
        return;
    }
    if sig == PLACED_SIG.load(Ordering::Relaxed) {
        gui_health_tick(&scan, &scan.cards, now);
    }
}

/// Draws the slot-1 label and the plane-loadout labels with the game's own text code (as the elevation readout
/// does): the font is a shared object, so its style is saved and put back.
unsafe extern "C" fn draw_marker() {
    draw_marker_text();
    draw_plane_labels();
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
    let y = if desk == 0 { f32::NEG_INFINITY } else { read::<f32>(desk + offset_of!(Body, position_y)) };
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

/// The plane buttons' labels, drawn the same way as the slot-1 text and under the same gates.
unsafe fn draw_plane_labels() {
    let _hook = diag::scope("draw_plane_labels");
    // The input/battle ticks validate screen lifetime before rendering. Do not scan/repair GUI ownership here.
    if !PRECOMBAT_LIVE.load(Ordering::Relaxed) || esc_menu_open() || doors_closed() {
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

unsafe fn draw_marker_text() {
    let _hook = diag::scope("draw_marker");
    if PRECOMBAT_LIVE.load(Ordering::Relaxed) && doors_closed() {
        return;
    }
    if !PRECOMBAT_LIVE.load(Ordering::Relaxed) || !MARKER_SHOW.load(Ordering::Relaxed) || esc_menu_open() {
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

/// Switches off every listed `PLANE_ARM_n` element whose number is not in `keep` (a stale one from an earlier card order).
unsafe fn hide_plane_buttons(keep: &[usize]) {
    let scan = scan_gui_list();
    if !scan.complete {
        warn_incomplete_scan(scan.visited, scan.limit);
    }
    // keep is computed only from a complete placement scan, or is empty on an explicit screen exit.
    // A partial exit scan can hide known members safely; it must not trigger an unbounded fallback search.
    for &element in &scan.elements {
        if read::<i32>(element + OFF_ACTIVE) != 2 {
            continue;
        }
        if let Some(number) = cstr(element + GUI_NAME).strip_prefix("PLANE_ARM_").and_then(|n| n.parse::<usize>().ok()) {
            if !keep.contains(&number) {
                set_gui_active(element, false);
            }
        }
    }
}

/// Hides only currently listed owned toggles. Scene teardown may already have freed retained addresses.
unsafe fn hide_toggles(why: &str) {
    PRECOMBAT_LIVE.store(false, Ordering::Relaxed);
    MARKER_SHOW.store(false, Ordering::Relaxed);
    hide_plane_buttons(&[]);
    if let Ok(mut labels) = PLANE_LABELS.lock() {
        labels.clear();
    }
    let scan = scan_gui_list();
    if !scan.complete {
        warn_incomplete_scan(scan.visited, scan.limit);
    }
    let mut hidden = 0;
    for (i, name) in TOGGLE_NAMES.iter().enumerate() {
        let gui = TOGGLE_GUIS[i].load(Ordering::Relaxed);
        if scan.elements.contains(&gui)
            && cstr(gui + GUI_NAME).as_bytes() == &name[..name.len() - 1]
            && read::<i32>(gui + OFF_ACTIVE) == 2
        {
            set_gui_active(gui, false);
            hidden += 1;
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
    let scan = scan_cards(true);
    // while the pre-combat screen is up the game refunds on every ammo pick; a partial list is treated as
    // "screen still up" so an uncertain scan cannot wipe the share the player just chose
    if PRECOMBAT_LIVE.load(Ordering::Relaxed) || !scan.complete || !scan.cards.is_empty() {
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
                read::<f32>(e + offset_of!(Body, position_x)),
                read::<f32>(e + offset_of!(Body, position_y)),
                read::<f32>(e + 0x218),
                read::<f32>(e + 0x21c),
                read::<usize>(e).wrapping_sub(IMAGE_BASE),
                read::<i32>(e + offset_of!(Gui, gui_i1)),
                read::<usize>(e + GUI_LINK_TARGET),
            ));
        }
        node = read(node);
    }
    log::info!("GUI dump #{dump} [{trigger}] {} elements: {}", lines.len(), lines.join(" ; "));
}

/// True when this button's previous accepted click is older than the debounce window; records `now` when so.
fn accept_click(last_click_ms: &AtomicU64, now: u64) -> bool {
    let last = last_click_ms.load(Ordering::Relaxed);
    if now.wrapping_sub(last) <= CLICK_DEBOUNCE_MS {
        return false;
    }
    last_click_ms.store(now, Ordering::Relaxed);
    true
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
        if button as u32 == 1 && (1..=54).contains(&number) && accept_click(&LAST_PLANE_CLICK_MS[number], now) {
            let ships = card_ship_list();
            // a column follows its ship while cards move: the button may sit over another card for a moment, but
            // it still changes the loadout of the ship it was painted for
            let painted = PAINTED_PLANE_SHIPS[card - 1].load(Ordering::Relaxed);
            let ship = if painted != 0 && ships.iter().any(|&(_, ship)| ship == painted) {
                Some(painted)
            } else {
                ships.get(card - 1).map(|&(_, ship)| ship)
            };
            if let Some(ship) = ship {
                let next = plane_loadouts::next_choice(ship, craft);
                plane_loadouts::set_choice(ship, craft, next);
                play_button_sound("plane loadout button");
                // the state changed: retarget the button's existing sprite (no new Sprite per click)
                if !paint_loadout(gui as *mut u8, ship, craft) {
                    set_gui_active(gui, false);
                }
                if let Ok(mut labels) = PLANE_LABELS.lock() {
                    if let Some(l) = labels.iter_mut().find(|l| l.0 == slot) {
                        let mut bytes = plane_loadouts::label_for(craft, next).into_bytes();
                        bytes.push(0);
                        l.4 = bytes;
                    }
                }
                toggle_line(format!("PLANE button clicked: slot {slot} ship {ship:#x} -> {:?}", next.map(|a| plane_loadouts::arms()[a].key.clone())));
            }
        }
        return true;
    }
    let Some(slot) = name.strip_prefix("ALLY_TOGGLE_").and_then(|n| n.parse::<usize>().ok()) else {
        return false;
    };
    // the game delivers one click twice (press and release): one toggle per CLICK_DEBOUNCE_MS
    let now = now_ms();
    if button as u32 == 1 && (2..=9).contains(&slot) && accept_click(&LAST_ALLY_CLICK_MS[slot], now) {
        let mask = SLOT_MASK.fetch_xor(1 << slot, Ordering::Relaxed) ^ (1 << slot);
        // the slot state changed: retarget the button's existing sprite and reload it
        if !paint_toggle(gui as *mut u8, slot) {
            set_gui_active(gui, false);
        }
        play_button_sound("ally toggle button");
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
    if which.starts_with("B2") && PRECOMBAT_UI_READY.load(Ordering::Relaxed) {
        crumb!("ensure_toggles: enter");
        ensure_toggles();
        crumb!("ensure_toggles: done");
    }
    if !diag::debug() {
        return;
    }
    let build = PRECOMBAT_BUILDS.fetch_add(1, Ordering::Relaxed);
    if build >= 24 && !diag::debug() {
        return;
    }
    let count: usize = read(GUI_COUNT);
    let scan = scan_gui_list();
    let mut names = Vec::new();
    let mut ok = 0usize;
    for &element in &scan.elements {
        let name = cstr(element + GUI_NAME);
        if name == "PRECOMBAT_OK" {
            ok = element;
        }
        names.push(format!("{name}@{element:#x}{}", if read::<i32>(element + OFF_ACTIVE) != 0 { "" } else { "(off)" }));
    }
    log::info!(
        "PRECOMBAT build #{build} [{which}]: GUI count {count}, {} listed (scan {}/{} complete {}): {}",
        names.len(),
        scan.visited,
        scan.limit,
        scan.complete,
        names.join(" ")
    );
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
    precombat_tick(1);
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
    let path = "Modloader/config/ally.json";
    let config: Config = match std::fs::read_to_string(path) {
        Ok(text) => parse_config(&text).unwrap_or_else(|error| {
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
    if let Ok(mut text) = MARKER_TEXT.lock() {
        // ASCII only; '@' is a line break for the game's text code
        *text = config.marker_text.bytes().filter(|b| (0x20..0x7f).contains(b) && *b != b'@').chain([0]).collect();
    }
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
    for (name, address, expected) in [
        ("gui new", GUI_NEW, &GUI_NEW_PROLOGUE[..]),
        ("gui ctor", GUI_CTOR, &GUI_CTOR_PROLOGUE[..]),
        ("gui setpos", GUI_SETPOS, &GUI_SETPOS_PROLOGUE[..]),
        ("gui setbg", GUI_SETBG, &GUI_SETBG_PROLOGUE[..]),
        ("gui sprite name", SET_SPRITE_NAME, &SET_SPRITE_NAME_PROLOGUE[..]),
        ("gui sprite clear", OBJECT_ARRAY_DELETE_ALL, &OBJECT_ARRAY_DELETE_ALL_PROLOGUE[..]),
        ("gui reset", GUI_RESET, &GUI_RESET_PROLOGUE[..]),
        ("gui find", GUI_FIND, &GUI_FIND_PROLOGUE[..]),
    ] {
        let found = std::slice::from_raw_parts(address as *const u8, expected.len());
        if found != expected {
            return Err(format!("{name} at {address:#x} has unexpected bytes {found:02x?}: toggle not installed"));
        }
    }
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
        PRECOMBAT_UI_READY.store(true, Ordering::Relaxed);
    }
    let bytes = std::slice::from_raw_parts(MARKER_SEAM as *const u8, 5);
    let target = (MARKER_SEAM + 5).wrapping_add(i32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as isize as usize);
    if bytes[0] == 0xe8 && target == FLUSH {
        let mut trampoline = Trampoline::new();
        trampoline.absolute_call(FLUSH as *const ());
        trampoline.preserved_call(draw_marker as *const (), &[], &[], ReturnType::None);
        trampoline.relative_jump(MARKER_SEAM + 5);
        session.detour_trampoline(MARKER_SEAM, 5, trampoline).map_err(|e| e.to_string())?;
    } else {
        log::warn!("hf_ally: render seam {MARKER_SEAM:#x} is not a call to the flush: not hooked");
    }
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
    GUI_INSPECT.store(config.logging.gui_inspect, Ordering::Relaxed);
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
    enemy_waves::install(&mut session, config.enemy_group_size, config.enemy_all_at_once);
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
    if reticle_scale != 0.0 || config.logging.debug_overlay {
        // the reticle module owns the single per-frame battle HUD post-pass that both features draw from
        ally_reticles::install(&mut session, ally_damage_color, reticle_scale, config.logging.debug_overlay);
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
