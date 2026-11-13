//! Pre-combat screen GUI lifecycle: ally toggles, plane-loadout buttons, labels and the
//! complete-list validation that guards their cached native pointers.
//!
//! Hook installation lives here as [`install`]; the callbacks and native patch sites it owns are
//! private implementation details.

use std::{
    mem::{offset_of, size_of},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Mutex,
    },
};

use patchy::{PatchSession, ReturnType, Trampoline};

use crate::battle::{emit_last_summary, restore_orders};
use crate::diag;
use crate::lifecycle_log::scene;
use crate::native::{plausible, read, GUI_ROOT, IMAGE_BASE, OFF_ACTIVE};
use crate::native_layout::{Body, Gui, Link, Node};
use crate::plane_loadouts;
use crate::sound;
use crate::time::now_ms;

/// ammoSelectButtonPress? (0x14026f880): rebuilds the native ammo selector after a press.
const AMMO_PRESS: usize = 0x1_4026_f880;
const AMMO_PRESS_STOLEN: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10];

/// Stores the configured slot-1 label the way the native render seam expects it: printable
/// ASCII only, `@` (the game's line break) removed, NUL-terminated.
pub(super) fn configure_marker(text: &str) {
    if let Ok(mut marker) = MARKER_TEXT.lock() {
        *marker = text
            .bytes()
            .filter(|b| (0x20..0x7f).contains(b) && *b != b'@')
            .chain([0])
            .collect();
    }
}

/// Installs the pre-combat GUI hooks: the three builder seams, the native GUI recipe checks, the
/// click/entry hooks, the render and input seams, and the optional ammo-press inspector hook.
/// Mandatory byte mismatches and patch-construction failures return an error; optional hook
/// conflicts log warnings. The caller owns committing the complete patch session.
///
/// # Safety
/// Call during serialized initialization after verifying the Steam 1.163 preferred image base.
/// Fixed native code sites must remain mapped and unchanged while their patches are staged.
pub(crate) unsafe fn install(session: &mut PatchSession, gui_inspect: bool) -> Result<(), String> {
    let sites: [(usize, [u8; 5], *const ()); 3] = [
        (
            PRECOMBAT_SEAM,
            PRECOMBAT_SEAM_BYTES,
            on_precombat as *const (),
        ),
        (
            PRECOMBAT_B1_SEAM,
            PRECOMBAT_B1_BYTES,
            on_precombat_b1 as *const (),
        ),
        (
            PRECOMBAT_B2_SEAM,
            PRECOMBAT_B2_BYTES,
            on_precombat_b2 as *const (),
        ),
    ];
    for (site, bytes, hook) in sites {
        let found = std::slice::from_raw_parts(site as *const u8, bytes.len());
        if found != bytes {
            return Err(format!(
                "pre-combat seam {site:#x} has unexpected bytes {found:02x?}"
            ));
        }
        let mut trampoline = Trampoline::new();
        trampoline.preserved_call(hook, &[], &[], ReturnType::None);
        trampoline.absolute_call(PRECOMBAT_SEAM_TARGET as *const ());
        trampoline.relative_jump(site + bytes.len());
        session
            .detour_trampoline(site, bytes.len(), trampoline)
            .map_err(|e| e.to_string())?;
    }
    for (name, address, expected) in [
        ("gui new", GUI_NEW, &GUI_NEW_PROLOGUE[..]),
        ("gui ctor", GUI_CTOR, &GUI_CTOR_PROLOGUE[..]),
        ("gui setpos", GUI_SETPOS, &GUI_SETPOS_PROLOGUE[..]),
        ("gui setbg", GUI_SETBG, &GUI_SETBG_PROLOGUE[..]),
        (
            "gui sprite name",
            SET_SPRITE_NAME,
            &SET_SPRITE_NAME_PROLOGUE[..],
        ),
        (
            "gui sprite clear",
            OBJECT_ARRAY_DELETE_ALL,
            &OBJECT_ARRAY_DELETE_ALL_PROLOGUE[..],
        ),
        ("gui reset", GUI_RESET, &GUI_RESET_PROLOGUE[..]),
        ("gui find", GUI_FIND, &GUI_FIND_PROLOGUE[..]),
    ] {
        let found = std::slice::from_raw_parts(address as *const u8, expected.len());
        if found != expected {
            return Err(format!(
                "{name} at {address:#x} has unexpected bytes {found:02x?}: toggle not installed"
            ));
        }
    }
    let found = std::slice::from_raw_parts(CLICK_ENTRY as *const u8, CLICK_STOLEN.len());
    if found != CLICK_STOLEN {
        log::warn!("hf_ally: ClickFunc entry is already hooked by another mod (hf_probe hook_clicks?): no toggle, no GUI inspector");
    } else {
        // entry hook that can swallow the click: call the hook with the four arguments, `ret` when it says yes
        let mut code: Vec<u8> = vec![
            0x51, 0x52, 0x41, 0x50, 0x41, 0x51, 0x48, 0x83, 0xec, 0x28, 0x48, 0xb8,
        ];
        code.extend((on_click_pre as *const () as usize as u64).to_le_bytes());
        code.extend([
            0xff, 0xd0, 0x48, 0x83, 0xc4, 0x28, 0x84, 0xc0, 0x41, 0x59, 0x41, 0x58, 0x5a, 0x59,
        ]);
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
    let target =
        (MARKER_SEAM + 5).wrapping_add(
            i32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as isize as usize,
        );
    if bytes[0] == 0xe8 && target == FLUSH {
        let mut trampoline = Trampoline::new();
        trampoline.absolute_call(FLUSH as *const ());
        trampoline.preserved_call(draw_marker as *const (), &[], &[], ReturnType::None);
        trampoline.relative_jump(MARKER_SEAM + 5);
        session
            .detour_trampoline(MARKER_SEAM, 5, trampoline)
            .map_err(|e| e.to_string())?;
    } else {
        log::warn!(
            "hf_ally: render seam {:#x} is not a call to the flush: not hooked",
            MARKER_SEAM
        );
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
    GUI_INSPECT.store(gui_inspect, Ordering::Relaxed);
    if gui_inspect {
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
    Ok(())
}

unsafe extern "C" fn on_ammo_press() {
    let _hook = diag::scope("on_ammo_press");
    inspect_gui("ammoSelectButtonPress");
}

/// Enables the plane-loadout buttons once their native hooks are installed.
pub(super) fn set_plane_buttons(enabled: bool) {
    PLANE_BUTTONS.store(enabled, Ordering::Relaxed);
}

/// The per-ship ally choices currently switched on (bit n = ship number n).
pub(super) fn selected_mask() -> u32 {
    SLOT_MASK.load(Ordering::Relaxed)
}

/// Replace the card choices for a test fight; Shipworks restores the campaign choices on return.
pub(crate) fn replace_selected_mask(mask: u32) -> u32 {
    SLOT_MASK.swap(mask, Ordering::Relaxed)
}

/// Consumes the "entered through the pre-combat screen" flag for the next battle.
pub(super) fn take_seen() -> bool {
    PRECOMBAT_SEEN.swap(false, Ordering::Relaxed)
}

/// Whether the pre-combat screen is currently serviced by this module.
pub(super) fn is_open() -> bool {
    PRECOMBAT_LIVE.load(Ordering::Relaxed)
}

/// The refund guard: true while the screen is open, or when the card scan is incomplete or still
/// reports player cards. An uncertain scan must not wipe an ammo share the player just chose.
///
/// # Safety
/// Call on the game thread while the native GUI registry and its listed elements remain valid
/// for this scan. An incomplete scan is not evidence that cached native objects are alive.
pub(super) unsafe fn has_cards_or_uncertain() -> bool {
    let scan = scan_cards(true);
    is_open() || !scan.complete || !scan.cards.is_empty()
}

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
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48,
    0x89, 0x7c, 0x24, 0x20,
];
/// `ObjectArray_DeleteAll(gui)` 0x14004bb10: calls each sprite's native deleting destructor (slot 0, the
/// `New(0x1a8)` pair) and sets the vector end back to its begin. Used only to normalize an element that
/// accumulated more than one background; safe in builder/input/tick callbacks, never from a render hook.
const OBJECT_ARRAY_DELETE_ALL: usize = 0x1_4004_bb10;
const OBJECT_ARRAY_DELETE_ALL_PROLOGUE: [u8; 11] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x56, 0x48, 0x83, 0xec, 0x20, 0x48,
];
const GUI_RESET: usize = 0x1_4015_cc30; // clears the +0xab8/+0xad8/+0xaf8/+0xb18 strings
const GUI_RESET_PROLOGUE: [u8; 9] = [0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9];
const GUI_FIND: usize = 0x1_401a_9170; // FindGuiElement(char*)
const GUI_FIND_PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10];

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
const INPUT_TASK_PROLOGUE: [u8; 11] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x48, 0x89, 0x70, 0x10,
];
const CLICK_ENTRY: usize = 0x1_4012_f120;
const CLICK_STOLEN: [u8; 26] = [
    0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d, 0xac,
    0x24, 0xf8, 0xd3, 0xff, 0xff, 0xb8, 0x08, 0x2d, 0x00, 0x00,
];
/// Constants the initializer of PRECOMBAT_OK is called with (vtable +0x2c0).
const INIT_F2: usize = 0x1_4807_5ce8; // f32 globals (set at run time)
const INIT_F3: usize = 0x1_4807_5cec;
const INIT_SCALE: usize = 0x1_4035_4698; // f32
const INIT_DIM_A: usize = 0x1_43a1_58fc; // i32
const INIT_DIM_B: usize = 0x1_43a1_3e5c; // i32
const INIT_STR: [usize; 5] = [
    0x1_4035_2962,
    0x1_4035_3b70,
    0x1_4035_3b60,
    0x1_4035_3b50,
    0x1_4035_3628,
];
const INIT_ONE: usize = 0x1_4035_24d8; // f32 1.0
const BG_MINUS_ONE: usize = 0x1_4035_24ec; // f32 -1.0

/// v0.9.2/0.9.3: the ally logic runs only in a battle that was entered through the pre-combat screen (the card
/// choices come from it). Re-engaged chases, interceptions and other battles play as vanilla.
static PRECOMBAT_SEEN: AtomicBool = AtomicBool::new(false);

/// Set once the pre-combat click hook is installed. Without it the buttons cannot respond, so they are not created.
static PRECOMBAT_UI_READY: AtomicBool = AtomicBool::new(false);
/// Per-ship ally choice: bit n = ship number n (2..=9, as numbered on the pre-combat screen) may be the ally.
static SLOT_MASK: AtomicU32 = AtomicU32::new(1 << 2);

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
type RenderTextFn = unsafe extern "win64" fn(
    *mut u8,
    f32,
    f32,
    u32,
    *const std::ffi::c_char,
    *mut f32,
    *mut f32,
    i32,
    f32,
) -> f32;
/// The buttons exist and are shown (set when they are placed, cleared when they are hidden).
static PRECOMBAT_LIVE: AtomicBool = AtomicBool::new(false);
// A partial exit scan can hide known members, but must retry until ownership is fully checked.
static PRECOMBAT_HIDE_PENDING: AtomicBool = AtomicBool::new(false);
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
static LOADOUT_GUIS: [std::sync::atomic::AtomicUsize; 54] =
    [const { std::sync::atomic::AtomicUsize::new(0) }; 54];
/// The ship each card slot's plane buttons were painted for. While cards move, a column follows the card that
/// holds this ship, not the slot number it is named after.
static PAINTED_PLANE_SHIPS: [std::sync::atomic::AtomicUsize; 9] =
    [const { std::sync::atomic::AtomicUsize::new(0) }; 9];
/// Gap (GUI units) between the ALLY button and the plane button above it.
const LOADOUT_GAP: f32 = 3.0;
const LABEL_FONT: &std::ffi::CStr = c"flash_large_shadow";
static TOGGLE_GUIS: [std::sync::atomic::AtomicUsize; 8] =
    [const { std::sync::atomic::AtomicUsize::new(0) }; 8];
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

static GUI_INSPECT: AtomicBool = AtomicBool::new(false);
static INSPECT_DUMPS: AtomicU32 = AtomicU32::new(0);
static INSPECT_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

static PRECOMBAT_BUILDS: AtomicU32 = AtomicU32::new(0);

/// A NUL-terminated ASCII name at `address`, at most 48 bytes, printable only.
const GUI_NAME_READ_LIMIT: usize = 48;

unsafe fn cstr(address: usize) -> String {
    (0..GUI_NAME_READ_LIMIT)
        .map(|i| read::<u8>(address + i))
        .take_while(|&b| (0x20..0x7f).contains(&b))
        .map(|b| b as char)
        .collect()
}

/// Compare a fixed GUI identifier with the same 48-byte printable-ASCII boundary as `cstr`,
/// without allocating a name for every element in a per-frame scan.
pub(crate) unsafe fn cstr_is(address: usize, expected: &[u8]) -> bool {
    if expected.len() > GUI_NAME_READ_LIMIT {
        return false;
    }
    for (index, &byte) in expected.iter().enumerate() {
        if read::<u8>(address + index) != byte {
            return false;
        }
    }
    expected.len() == GUI_NAME_READ_LIMIT
        || !(0x20..0x7f).contains(&read::<u8>(address + expected.len()))
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
pub(crate) type GuiInitFn = unsafe extern "C" fn(
    *mut u8,
    *const u8,
    f32,
    f32,
    f32,
    f32,
    *const u8,
    *const u8,
    *const u8,
    *const u8,
    *const u8,
    f32,
);
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
pub(crate) struct GuiScan {
    pub(crate) elements: Vec<usize>,
    /// False when the walk hit the cap before the sentinel: the list is only partially known.
    pub(crate) complete: bool,
    visited: usize,
    limit: usize,
}

pub(crate) unsafe fn scan_gui_list() -> GuiScan {
    let head: usize = read(GUI_LIST);
    let count: usize = read(GUI_COUNT);
    let limit = count
        .saturating_add(GUI_SCAN_SLACK)
        .clamp(GUI_SCAN_MIN, GUI_SCAN_MAX);
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
pub(crate) unsafe fn set_gui_active(gui: usize, active: bool) {
    if gui == 0 {
        return;
    }
    (gui as *mut i32)
        .byte_add(OFF_ACTIVE)
        .write_unaligned(if active { 2 } else { 0 });
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
pub(crate) unsafe fn load_gui(gui: usize) {
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
    let Some(sprite) = newest_sprite(gui) else {
        return 0;
    };
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
        let sprite = set_bg(
            gui as *mut u8,
            0,
            name.as_ptr(),
            read::<f32>(BG_MINUS_ONE),
            0,
            -1,
        );
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
        let is_card = cstr_is(element + GUI_NAME, b"PRECOMBAT_A");
        let is_other = !quiet
            && [b"PRECOMBAT_B".as_slice(), b"MINIFIG_NUMBER", b"MES_APP"]
                .iter()
                .any(|name| cstr_is(element + GUI_NAME, name));
        if !is_card && !is_other {
            continue;
        }
        let entry = (
            read::<f32>(element + offset_of!(Body, position_x)),
            read::<f32>(element + offset_of!(Body, position_y)),
            read::<f32>(element + 0x218),
            read::<f32>(element + 0x21c),
        );
        if is_card {
            cards.push(entry);
        } else {
            let name = cstr(element + GUI_NAME);
            other.push(format!(
                "{name} ({:.0},{:.0} {:.0}x{:.0})",
                entry.0, entry.1, entry.2, entry.3
            ));
        }
    }
    cards.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    if !quiet {
        toggle_line(format!(
            "ALLY cards: {} ship cards {:?} | others {} | scan {}/{} complete {}",
            cards.len(),
            cards
                .iter()
                .map(|c| format!("({:.0},{:.0} {:.0}x{:.0})", c.0, c.1, c.2, c.3))
                .collect::<Vec<_>>(),
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
    let gui = allocate_gui();
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
    attach_gui(gui);
    gui
}

/// Allocate/construct native GUI storage, without initializing a control recipe.
///
/// # Safety
/// Game-thread callback with a live global root; caller must initialize and attach the new GUI.
pub(crate) unsafe fn allocate_gui() -> *mut u8 {
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
    ctor(mem, 0)
}

/// Transfer a freshly constructed/initialized control into the native GUI root.
///
/// # Safety
/// The pointer must come from allocate_gui in this callback, never a detached or cached object.
pub(crate) unsafe fn attach_gui(gui: *mut u8) {
    let root: usize = read(GUI_ROOT);
    let node_vtable: usize = read(root);
    let add: NodeAddFn = std::mem::transmute(read::<usize>(node_vtable + 0x50));
    add(root as *mut u8, gui, 0, 0);
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
pub(crate) unsafe fn get_or_make(
    name: &[u8],
    kept_slot: &std::sync::atomic::AtomicUsize,
    listed: &[usize],
    complete: bool,
) -> Option<(usize, bool)> {
    get_or_make_with(name, kept_slot, listed, complete, || make_gui(name))
}

/// Same guarded reuse as get_or_make, with a feature-specific native constructor recipe.
///
/// # Safety
/// The list must belong to this game-thread callback. `create` must return a freshly native-
/// constructed, initialized, root-attached GUI with the requested name, or null on failure.
pub(crate) unsafe fn get_or_make_with(
    name: &[u8],
    kept_slot: &std::sync::atomic::AtomicUsize,
    listed: &[usize],
    complete: bool,
    create: impl FnOnce() -> *mut u8,
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
    let ptr = create();
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
        if read::<i32>(element + OFF_ACTIVE) != 0 && cstr_is(element + GUI_NAME, b"PRECOMBAT_A") {
            out.push((
                read::<f32>(element + offset_of!(Body, position_x)),
                plane_loadouts::ship_of_link(read::<usize>(element + GUI_LINK_TARGET)),
            ));
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
    let sprite = if plane_loadouts::choice(ship, craft).is_some() {
        plane_loadouts::SPRITE_CHOSEN
    } else {
        plane_loadouts::SPRITE_DEFAULT
    };
    paint_button(gui as usize, sprite)
}

/// The ship carries plane type `craft` and the player can still choose an armament for it (or already did): it gets a button.
unsafe fn loadout_wanted(ship: usize, craft: usize) -> bool {
    plane_loadouts::has_craft(ship, craft)
        && (!plane_loadouts::available_arms(ship, craft).is_empty()
            || plane_loadouts::choice(ship, craft).is_some())
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
            toggle_line(format!(
                "PLANE button card {card} hidden: {}",
                plane_loadouts::describe(s)
            ));
        } else if card <= 3 {
            toggle_line(format!(
                "PLANE button card {card} hidden: no ship link on this card"
            ));
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
    set_plane_label(
        slot,
        Some((
            pos[0],
            pos[1],
            size,
            plane_loadouts::label_for(craft, plane_loadouts::choice(ship, craft)),
        )),
    );
    toggle_line(format!(
        "PLANE button {slot}: ship {ship:#x} {} at ({:.0},{:.0}) size {size:.0}, choice {:?}",
        plane_loadouts::crafts()[craft].label,
        pos[0],
        pos[1],
        plane_loadouts::choice(ship, craft).map(|a| plane_loadouts::arms()[a].key.clone())
    ));
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
    let size = gap.map_or(TOGGLE_SIZE, |gap| {
        (gap - NUMBER_WIDTH).clamp(TOGGLE_MIN_SIZE, TOGGLE_SIZE)
    });
    Some((x, dx, size))
}

/// Moves an already placed element and resizes its hit box. The sprite is drawn from the element's position every
/// frame, so no resource reload is needed while following moving cards.
unsafe fn move_gui(gui: usize, pos: [f32; 2], size: f32) {
    move_gui_rect(gui, pos, [size, size]);
}

/// Move a freshly created or currently validated native GUI and set its rectangular hit box.
///
/// # Safety
/// Same game-thread and live-membership requirements as `load_gui`.
pub(crate) unsafe fn move_gui_rect(gui: usize, pos: [f32; 2], size: [f32; 2]) {
    let set_pos: SetPosFn = std::mem::transmute(GUI_SETPOS);
    set_pos(gui as *mut u8, pos.as_ptr());
    (gui as *mut f32).byte_add(0x218).write_unaligned(size[0]);
    (gui as *mut f32).byte_add(0x21c).write_unaligned(size[1]);
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
    let ships = if PLANE_BUTTONS.load(Ordering::Relaxed) {
        card_ships_from(&scan.elements)
    } else {
        Vec::new()
    };
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
        let Some((gui, created)) = get_or_make(name, kept_slot, &scan.elements, scan.complete)
        else {
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
                        place_loadout(
                            slot,
                            craft,
                            None,
                            [0.0, 0.0],
                            0.0,
                            &scan.elements,
                            scan.complete,
                        );
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
                let pos = [
                    x + dx + size + LOADOUT_GAP * 2.0,
                    GRID_Y + row as f32 * (psize + LOADOUT_GAP),
                ];
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
            let (active, unloaded, detached, invisible, abandoned) =
                owned_sprite_summary(&final_scan.elements);
            toggle_line(format!(
                "ALLY GUI build {build_epoch}: {active} owned elements active, {unloaded} without a loaded sprite, {detached} detached, {invisible} opacity 0, {abandoned} retained but unlisted"
            ));
        }
    }
    toggle_line(format!(
        "ALLY placed (slot@card x+dx/size): {}",
        layout.join(" ")
    ));
}

/// Moves the plane buttons of `slot` that are shown, and their labels. Rows are counted in the same order as the
/// placement, so a missing type does not collapse the rows into each other while the cards move.
unsafe fn follow_plane_buttons(listed: &[usize], slot: usize, x: f32, dx: f32, size: f32) {
    let mut row = 0usize;
    for craft in 0..plane_loadouts::crafts().len() {
        let number = craft * 9 + slot;
        let gui = LOADOUT_GUIS[number - 1].load(Ordering::Relaxed);
        if !listed.contains(&gui)
            || !gui_reusable(gui, LOADOUT_NAMES[number - 1])
            || read::<i32>(gui + OFF_ACTIVE) != 2
        {
            continue;
        }
        let pos = [
            x + dx + size + LOADOUT_GAP * 2.0,
            GRID_Y + row as f32 * (size + LOADOUT_GAP),
        ];
        move_gui(gui, pos, size);
        move_plane_label(number, pos[0], pos[1], size);
        row += 1;
    }
}

/// Moves each plane column with the card that holds the ship it was painted for. The column keeps the slot-based
/// button name it was built with; the full placement re-keys it to the new slot after the cards rest. Following
/// the ship (not the slot) keeps the column attached to the carrier while two cards cross.
unsafe fn follow_plane_columns(
    cards: &[(f32, f32, f32, f32)],
    listed: &[usize],
    ships: &[(f32, usize)],
) {
    if !PLANE_BUTTONS.load(Ordering::Relaxed) {
        return;
    }
    for (index, &(_, ship)) in ships.iter().enumerate() {
        if ship == 0 {
            continue;
        }
        let Some(painted_slot) = (1..=9usize)
            .find(|&slot| PAINTED_PLANE_SHIPS[slot - 1].load(Ordering::Relaxed) == ship)
        else {
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
unsafe fn follow_toggles(
    cards: &[(f32, f32, f32, f32)],
    listed: &[usize],
    ships: &[(f32, usize)],
    layout: u64,
) -> bool {
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
        if !listed.contains(&gui) || !gui_reusable(gui, name) || read::<i32>(gui + OFF_ACTIVE) != 2
        {
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
unsafe fn screen_gone(scan: &CardScan) -> bool {
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
    hide("screen gone");
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
        issues.push(format!(
            "card scan incomplete ({}/{})",
            scan.visited, scan.limit
        ));
        return issues;
    }
    let expected_slots = cards.len().min(9);
    for slot in 2..=expected_slots {
        if let Some(why) = gui_problem(
            TOGGLE_GUIS[slot - 2].load(Ordering::Relaxed),
            TOGGLE_NAMES[slot - 2],
            &scan.elements,
        ) {
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
            let ship = if slot <= expected_slots {
                ships.get(slot - 1).map(|s| s.1)
            } else {
                None
            };
            for craft in 0..plane_loadouts::crafts().len() {
                let number = craft * 9 + slot;
                let gui = LOADOUT_GUIS[number - 1].load(Ordering::Relaxed);
                let wanted = ship.is_some_and(|s| loadout_wanted(s, craft));
                let problem = if wanted {
                    gui_problem(gui, LOADOUT_NAMES[number - 1], &scan.elements)
                } else if scan.elements.contains(&gui)
                    && cstr(gui + GUI_NAME).as_bytes()
                        == &LOADOUT_NAMES[number - 1][..LOADOUT_NAMES[number - 1].len() - 1]
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
    let summary = format!(
        "{}{}",
        shown.join("; "),
        if more > 0 {
            format!(" (+{more} more)")
        } else {
            String::new()
        }
    );
    if attempt > GUI_HEALTH_TRIES {
        GUI_RECOVERY_EXHAUSTED.store(true, Ordering::Relaxed);
        toggle_line(format!(
            "ALLY GUI health: repair budget of {GUI_HEALTH_TRIES} used, staying quiet: {summary}"
        ));
        return;
    }
    toggle_line(format!(
        "ALLY GUI health repair {attempt}/{GUI_HEALTH_TRIES}: {summary}"
    ));
    ensure_toggles();
}

/// Per-frame input/battle-manager tick; `source` is 0 for input, 1 for the battle manager.
///
/// # Safety
/// Call on the game thread outside rendering, with `source` in 0..=1, after native GUI checks.
/// Registry membership is callback-local; no concurrent native teardown may race traversal.
pub(super) unsafe fn tick(source: usize) {
    if !PRECOMBAT_LIVE.load(Ordering::Relaxed) {
        if PRECOMBAT_HIDE_PENDING.load(Ordering::Relaxed) {
            hide("pending screen exit");
        }
        return;
    }
    let scan = scan_cards(true);
    if screen_gone(&scan) {
        return;
    }
    if TICKS_SEEN[source].fetch_add(1, Ordering::Relaxed) == 0 {
        toggle_line(format!("ALLY precombat tick: first call from hook {source} (0 = input task, 1 = battle manager)"));
    }
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
    let ships = if PLANE_BUTTONS.load(Ordering::Relaxed) {
        card_ships_from(&scan.elements)
    } else {
        Vec::new()
    };
    let sig = card_signature(cards, &ships);
    let now = now_ms();
    if layout != TICK_SIG.swap(layout, Ordering::Relaxed) {
        TICK_CHANGED_MS.store(now, Ordering::Relaxed);
        if diag::debug() && TICK_LOGS.fetch_add(1, Ordering::Relaxed) < 4000 {
            log::info!(
                "ALLY card layout moving: {:?}",
                cards
                    .iter()
                    .map(|c| format!("{:.1}", c.0))
                    .collect::<Vec<_>>()
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
    if sig != PLACED_SIG.load(Ordering::Relaxed)
        && now.wrapping_sub(TICK_CHANGED_MS.load(Ordering::Relaxed)) >= SETTLE_MS
    {
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
    let y = if desk == 0 {
        f32::NEG_INFINITY
    } else {
        read::<f32>(desk + offset_of!(Body, position_y))
    };
    let state = (hold as u32) | (((y / 50.0) as i32 as u32) << 1);
    if state != DOOR_STATE.swap(state, Ordering::Relaxed)
        && DOOR_LOGS.fetch_add(1, Ordering::Relaxed) < 400
    {
        toggle_line(format!(
            "ALLY roller cover: ROLL_DESK y={y:.0} label held back={hold} t+{since_entry}ms"
        ));
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
    let Ok(labels) = PLANE_LABELS.try_lock().map(|l| l.clone()) else {
        return;
    };
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
    let (old_scale, old_color, old_blend): (f32, u32, i32) = (
        read(base + 0x100c),
        read(base + 0x1020),
        read(base + 0x1028),
    );
    let style: StyleFn = std::mem::transmute(STYLE_FONT);
    style(LABEL_FONT.as_ptr(), 1.0, 2, LABEL_COLOR);
    let render: RenderTextFn = std::mem::transmute(FONT_RENDER);
    for (_, bx, by, size, text) in labels {
        // right of the round button, vertically centred on it
        // whole pixels only: a bitmap font drawn between pixels is blurred
        let (x, y) = (
            (960.0 + bx + size * 0.5 + 6.0).round(),
            (540.0 + by - 8.0).round(),
        );
        render(
            font,
            x,
            y,
            0,
            text.as_ptr() as *const _,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            0.0,
        ); // align 0 = left edge at x
    }
    style(LABEL_FONT.as_ptr(), old_scale, old_blend, old_color);
}

/// Draws the configured player label, restoring the shared native font's style afterward.
///
/// # Safety
/// Call only at the verified render seam on the game thread, after the input/battle tick has
/// serviced screen lifetime. Native font and GUI resources must remain valid for this draw.
pub(crate) unsafe fn draw_marker_text() {
    let _hook = diag::scope("draw_marker");
    if PRECOMBAT_LIVE.load(Ordering::Relaxed) && doors_closed() {
        return;
    }
    if !PRECOMBAT_LIVE.load(Ordering::Relaxed)
        || !MARKER_SHOW.load(Ordering::Relaxed)
        || esc_menu_open()
    {
        return;
    }
    let Ok(text) = MARKER_TEXT.lock().map(|t| t.clone()) else {
        return;
    };
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
    let (old_scale, old_color, old_blend): (f32, u32, i32) = (
        read(base + 0x100c),
        read(base + 0x1020),
        read(base + 0x1028),
    );
    let style: StyleFn = std::mem::transmute(STYLE_FONT);
    style(MARKER_FONT.as_ptr(), MARKER_SCALE, 2, MARKER_COLOR);
    // GUI units have the origin in the middle of the 1920 x 1080 frame; text coordinates start at the top left
    // whole pixels (a bitmap font between pixels is blurred); PLAYER_NUDGE brings it closer to the ship number
    let x = (960.0 + f32::from_bits(MARKER_X.load(Ordering::Relaxed)) + PLAYER_NUDGE).round();
    let y = (540.0 + GRID_Y - 11.0 * MARKER_SCALE).round();
    let render: RenderTextFn = std::mem::transmute(FONT_RENDER);
    render(
        font,
        x,
        y,
        2,
        text.as_ptr() as *const _,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        0,
        0.0,
    );
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
    crate::shipworks::tick();
    tick(0);
}

/// Switches off every listed `PLANE_ARM_n` element whose number is not in `keep` (a stale one from an earlier card order).
unsafe fn hide_plane_buttons(keep: &[usize]) {
    let scan = scan_gui_list();
    if !scan.complete {
        warn_incomplete_scan(scan.visited, scan.limit);
    }
    hide_plane_buttons_from(&scan.elements, keep);
}

/// Hide only members established by this callback's native GUI list scan.
unsafe fn hide_plane_buttons_from(listed: &[usize], keep: &[usize]) {
    // keep is computed only from a complete placement scan, or is empty on an explicit screen exit.
    // A partial exit scan can hide known members safely; it must not trigger an unbounded fallback search.
    for &element in listed {
        if read::<i32>(element + OFF_ACTIVE) != 2 {
            continue;
        }
        if let Some(number) = cstr(element + GUI_NAME)
            .strip_prefix("PLANE_ARM_")
            .and_then(|n| n.parse::<usize>().ok())
        {
            if !keep.contains(&number) {
                set_gui_active(element, false);
            }
        }
    }
}

/// Hides only currently listed owned toggles. Scene teardown may already have freed retained addresses.
///
/// # Safety
/// Call on the game thread outside rendering while the registry and listed GUI fields are stable.
/// Only this call's observed members may be mutated; retained addresses alone are not authority.
pub(super) unsafe fn hide(why: &str) {
    let was_live = PRECOMBAT_LIVE.swap(false, Ordering::Relaxed);
    if !was_live && !PRECOMBAT_HIDE_PENDING.load(Ordering::Relaxed) {
        return;
    }
    MARKER_SHOW.store(false, Ordering::Relaxed);
    if let Ok(mut labels) = PLANE_LABELS.lock() {
        labels.clear();
    }
    let scan = scan_gui_list();
    if !scan.complete {
        warn_incomplete_scan(scan.visited, scan.limit);
    }
    PRECOMBAT_HIDE_PENDING.store(!scan.complete, Ordering::Relaxed);
    hide_plane_buttons_from(&scan.elements, &[]);
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

/// Logs every active GUI element: name, class, position, size, value fields. Throttled, at most 80 dumps.
unsafe fn inspect_gui(trigger: &str) {
    let now = now_ms();
    if now.wrapping_sub(INSPECT_MS.load(Ordering::Relaxed)) < 700
        || INSPECT_DUMPS.load(Ordering::Relaxed) >= 80
    {
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
    log::info!(
        "GUI dump #{dump} [{trigger}] {} elements: {}",
        lines.len(),
        lines.join(" ; ")
    );
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
    if crate::shipworks::on_click(&name, button) {
        return true;
    }
    crumb!(
        "CLICK {name} gui={gui:#x} button={button} id={_id:#x} {}",
        scene()
    );
    restore_orders("click", false);
    diag::snapshot(format_args!("click {name}"));
    if GUI_INSPECT.load(Ordering::Relaxed) {
        inspect_gui(&format!("click {name}"));
    }
    // the end-of-battle screen and the way out of the game: the summary of the battle that just ended
    // (its result code is still fresh here); the battle hook never sees a real fight end
    if name.starts_with("RETRY_") || name.starts_with("QUIT_") {
        emit_last_summary();
        return false;
    }
    if name == "PRECOMBAT_OK" || name == "PRECOMBAT_BACK" {
        if name == "PRECOMBAT_BACK" {
            PRECOMBAT_SEEN.store(false, Ordering::Relaxed);
        }
        hide(&name);
        return false;
    }
    if let Some(number) = name
        .strip_prefix("PLANE_ARM_")
        .and_then(|n| n.parse::<usize>().ok())
    {
        let now = now_ms();
        let (craft, card) = ((number.max(1) - 1) / 9, (number.max(1) - 1) % 9 + 1);
        let slot = number; // the label key
        if button as u32 == 1
            && (1..=54).contains(&number)
            && accept_click(&LAST_PLANE_CLICK_MS[number], now)
        {
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
                sound::button("plane loadout button");
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
                toggle_line(format!(
                    "PLANE button clicked: slot {slot} ship {ship:#x} -> {:?}",
                    next.map(|a| plane_loadouts::arms()[a].key.clone())
                ));
            }
        }
        return true;
    }
    let Some(slot) = name
        .strip_prefix("ALLY_TOGGLE_")
        .and_then(|n| n.parse::<usize>().ok())
    else {
        return false;
    };
    // the game delivers one click twice (press and release): one toggle per CLICK_DEBOUNCE_MS
    let now = now_ms();
    if button as u32 == 1 && (2..=9).contains(&slot) && accept_click(&LAST_ALLY_CLICK_MS[slot], now)
    {
        let mask = SLOT_MASK.fetch_xor(1 << slot, Ordering::Relaxed) ^ (1 << slot);
        // the slot state changed: retarget the button's existing sprite and reload it
        if !paint_toggle(gui as *mut u8, slot) {
            set_gui_active(gui, false);
        }
        sound::button("ally toggle button");
        toggle_line(format!(
            "ALLY toggle clicked: ship {slot} {}",
            if mask & (1 << slot) != 0 { "ON" } else { "OFF" }
        ));
    }
    true
}

unsafe fn precombat_dump(which: &str) {
    crumb!("PRECOMBAT seam {which} {}", scene());
    restore_orders("pre-combat screen", false);
    diag::snapshot(format_args!("precombat {which}"));
    // the screen before a fight: the previous battle is over, log its summary now if the battle hook never saw it end
    emit_last_summary();
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
        names.push(format!(
            "{name}@{element:#x}{}",
            if read::<i32>(element + OFF_ACTIVE) != 0 {
                ""
            } else {
                "(off)"
            }
        ));
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
        let floats: Vec<String> = (0x100..0x140)
            .step_by(4)
            .map(|o| format!("{o:#x}={:.1}", read::<f32>(ok + o)))
            .collect();
        let sizes: Vec<String> = (0x210..0x224)
            .step_by(4)
            .map(|o| format!("{o:#x}={:.1}", read::<f32>(ok + o)))
            .collect();
        log::info!(
            "PRECOMBAT OK button c={ok:#x} vtable_rva={:#x} active={} floats {} | {}",
            read::<usize>(ok).wrapping_sub(IMAGE_BASE),
            read::<i32>(ok + OFF_ACTIVE),
            floats.join(" "),
            sizes.join(" "),
        );
    }
}
