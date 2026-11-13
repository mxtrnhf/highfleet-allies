//! Session-owned Shipworks test roster. Native designs are serialized snapshots, never editor
//! pointers. The existing FIGHT builder transfers fresh native copies into its player fleet.
//! Native evidence and manual validation checklist: docs/shipworks-test-allies.md.

mod design;

use std::{
    ffi::{CStr, CString},
    fmt::Write as _,
    mem::offset_of,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        Mutex,
    },
};

use patchy::{Condition, PatchSession, ReturnType, Trampoline};

use crate::{
    diag,
    native::{read, OFF_ACTIVE},
    native_layout::{Body, Gui},
    precombat, sound,
    time::now_ms,
};

/// Nine native player cards: the editor ship plus eight allies.
const ROSTER_LIMIT: usize = 8;
/// Enemy test roster mirrors the ally limit; both lists render in the same column.
const ENEMY_LIMIT: usize = 8;
/// GUI identifiers must stay within the 15-char inline (SSO) string storage: `cstr` reads the
/// identifier bytes in place, so a longer heap-allocated name would never compare equal.
const ADD_NAME: &[u8] = b"SW_ALLY_ADD\0";
const ENEMY_ADD_NAME: &[u8] = b"SW_ENEMY_ADD\0";
const CLEAR_NAME: &[u8] = b"SW_ALLY_CLEAR\0";
const CRASH_NAME: &[u8] = b"SW_ALLY_CRASH\0";
const ROW_NAMES: [&[u8]; ROSTER_LIMIT] = [
    b"SW_ALLY_1\0",
    b"SW_ALLY_2\0",
    b"SW_ALLY_3\0",
    b"SW_ALLY_4\0",
    b"SW_ALLY_5\0",
    b"SW_ALLY_6\0",
    b"SW_ALLY_7\0",
    b"SW_ALLY_8\0",
];
const ENEMY_ROW_NAMES: [&[u8]; ENEMY_LIMIT] = [
    b"SW_ENEMY_1\0",
    b"SW_ENEMY_2\0",
    b"SW_ENEMY_3\0",
    b"SW_ENEMY_4\0",
    b"SW_ENEMY_5\0",
    b"SW_ENEMY_6\0",
    b"SW_ENEMY_7\0",
    b"SW_ENEMY_8\0",
];
const STATUS_NAME: &[u8] = b"SW_ALLY_STATUS\0";
const ENEMY_STATUS_NAME: &[u8] = b"SW_ENEMY_STAT\0";
const BUTTON_GAP: f32 = 12.0;
const ROW_HEIGHT: f32 = 24.0;
const LIST_WIDTH: f32 = 480.0;
const CLICK_DEBOUNCE_MS: u64 = 120;
/// ARGB; red-dish hue marks the enemy list against the ally list's native colour.
const ENEMY_TEXT_COLOR: u32 = 0xffff_5050;
const BUTTON_STYLE: ControlStyle = ControlStyle {
    button: true,
    color: None,
};
const ROW_STYLE: ControlStyle = ControlStyle {
    button: false,
    color: None,
};
const ENEMY_ROW_STYLE: ControlStyle = ControlStyle {
    button: false,
    color: Some(ENEMY_TEXT_COLOR),
};
/// Creature side (+0xad4): positive is the player side, negative is the enemy side.
const SIDE_ALLY: f32 = 1.0;
const SIDE_ENEMY: f32 = -1.0;
const EDITOR_MODE: usize = 0x1_403e_f36a;
const TEST_FIGHT_FLAG: usize = 0x1_47fc_6fb6;
/// FIGHT: first ship is already attached to RDI; replay MOV RSI,[RDI+0x100] after adding ours.
const FLEET_SEAM: usize = 0x1_4013_dcc2;
const FLEET_BYTES: [u8; 7] = [0x48, 0x8b, 0xb7, 0x00, 0x01, 0x00, 0x00];
/// FIGHT: CALL Fleet_CreateEscadraFromShips building the enemy Escadra from generated names.
const ENEMY_FLEET_SEAM: usize = 0x1_4013_df7e;
const ENEMY_FLEET_BYTES: [u8; 5] = [0xe8, 0xdd, 0x90, 0x08, 0x00];
const CREATE_ESCADRA: usize = 0x1_401c_7060;
const CREATE_ESCADRA_BYTES: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57,
];
/// FIGHT: after the enemy Escadra's init call; RBX is the enemy Escadra, replay the LEA.
const ENEMY_APPEND_SEAM: usize = 0x1_4013_df8e;
const ENEMY_APPEND_BYTES: [u8; 7] = [0x48, 0x8d, 0x0d, 0x2b, 0x7a, 0x8d, 0x03];
/// preserved_call pushes 7 registers and subs 0x88, so at the callback setup the seam's
/// original stack args [S+0x20]/[S+0x28] sit at [RSP+0xE0]/[RSP+0xE8]; the callback reads its
/// own 5th/6th args from [RSP+0x20]/[RSP+0x28]. Writing there only displaces the saved XMM0,
/// which is dead after the replaced CALL.
const ENEMY_FLEET_SETUP: [u8; 26] = [
    0x48, 0x8b, 0x84, 0x24, 0xe0, 0x00, 0x00, 0x00, // MOV RAX, [RSP+0xE0]
    0x48, 0x89, 0x44, 0x24, 0x20, // MOV [RSP+0x20], RAX
    0x48, 0x8b, 0x84, 0x24, 0xe8, 0x00, 0x00, 0x00, // MOV RAX, [RSP+0xE8]
    0x48, 0x89, 0x44, 0x24, 0x28, // MOV [RSP+0x28], RAX
];
const LEVEL_ENTRY: usize = 0x1_4025_afd0;
const LEVEL_BYTES: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18];
const COPY_STRING: usize = 0x1_4000_56e0;
const COPY_STRING_BYTES: [u8; 12] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x57,
];
/// Get_Profile_(): selects the active profile. Its escadra vector (CVec of 8-byte pointers)
/// sits at profile+0x1f8 (begin) / +0x200 (end); the Combat level loader re-links every profile
/// escadra into the enemy fleet list, so a test enemy Escadra left there accumulates into the
/// next test battle.
const GET_PROFILE: usize = 0x1_4029_2a90;
const GET_PROFILE_BYTES: [u8; 16] = [
    0x0f, 0xb6, 0x15, 0xea, 0x44, 0xd3, 0x07, 0x0f, 0xb6, 0x0d, 0xe2, 0x44, 0xd3, 0x07, 0xe9, 0x0d,
];
const PROFILE_ESCADRAS: usize = 0x1f8;
/// Sanity bound for the profile escadra vector during the prune scan.
const ESCADRA_VECTOR_LIMIT: usize = 0x8000;
/// Enemy-side fleet list: vector of 0x18-byte link nodes (begin +0x0, end +0x8, cap +0x10);
/// the escadra pointer sits at node+0x8. The pre-combat screen builds PRECOMBAT_B cards from it.
const ENEMY_LIST: usize = 0x1_43a1_59c0;
const LINK_NODE_STRIDE: usize = 0x18;
/// Link_ClearVector: unlinks every node of a link vector (null-safe) and resets end=begin.
const LINK_CLEAR_VECTOR: usize = 0x1_400c_2400;
const LINK_CLEAR_VECTOR_BYTES: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48,
];
/// Combat level loader (SomethingWithLevelSerias) walks the enemy fleet list and dereferences
/// each node's escadra without a null check: `MOV RAX,[R14+8]; MOV RSI,[RAX+0x100]`. A node whose
/// escadra was destroyed (e.g. a previous test battle's fleet) crashes here reading 0x100.
/// The guard skips null-escadra nodes exactly like the loop's own advance at 0x1402c3a63.
const ENEMY_LOOP_SEAM: usize = 0x1_402c_3a00;
const ENEMY_LOOP_BYTES: [u8; 11] = [
    0x49, 0x8b, 0x46, 0x08, // MOV RAX, [R14+0x8]
    0x48, 0x8b, 0xb0, 0x00, 0x01, 0x00, 0x00, // MOV RSI, [RAX+0x100]
];
const ENEMY_LOOP_BODY: usize = 0x1_402c_3a0b; // resumes after the two MOVs
const ENEMY_LOOP_EXIT: usize = 0x1_402c_3a6c; // loop exit when R14 reaches R12
/// Ship count of an Escadra, as read by the pre-combat card builder (escadra+0x56c).
const ESCADRA_SHIP_COUNT: usize = 0x56c;
/// Sanity bound for list scans in diagnostics/prune.
const LIST_ENTRY_LIMIT: usize = 64;

struct Entry {
    name: CString,
    path: CString,
}

#[derive(Clone, Copy)]
struct ControlGeometry {
    position: [f32; 2],
    dimensions: [f32; 2],
}

#[derive(Clone, Copy)]
struct ControlStyle {
    button: bool,
    /// Overrides the template text colour (the enemy list's red-dish hue).
    color: Option<u32>,
}

struct Session {
    entries: Vec<Entry>,
    enemy_entries: Vec<Entry>,
    status: Option<CString>,
    enemy_status: Option<CString>,
    serial: u64,
    last_add_ms: Option<u64>,
    last_enemy_add_ms: Option<u64>,
    last_clear_ms: Option<u64>,
    saved_mask: Option<u32>,
    /// The enemy Escadra created by the last custom FIGHT, so it can be pruned from the
    /// profile's escadra vector before the next battle (it would otherwise accumulate).
    enemy_escadra: usize,
}

static SESSION: Mutex<Session> = Mutex::new(Session {
    entries: Vec::new(),
    enemy_entries: Vec::new(),
    status: None,
    enemy_status: None,
    serial: 0,
    last_add_ms: None,
    last_enemy_add_ms: None,
    last_clear_ms: None,
    saved_mask: None,
    enemy_escadra: 0,
});
static READY: AtomicBool = AtomicBool::new(false);
static CONTROLS_VISIBLE: AtomicBool = AtomicBool::new(false);
static TEST_ALLIES: AtomicU32 = AtomicU32::new(0);
static ADD_GUI: AtomicUsize = AtomicUsize::new(0);
static ENEMY_ADD_GUI: AtomicUsize = AtomicUsize::new(0);
static CLEAR_GUI: AtomicUsize = AtomicUsize::new(0);
static CRASH_GUI: AtomicUsize = AtomicUsize::new(0);
static CRASH_BUTTON: AtomicBool = AtomicBool::new(false);
static ROW_GUIS: [AtomicUsize; ROSTER_LIMIT] = [const { AtomicUsize::new(0) }; ROSTER_LIMIT];
static ENEMY_ROW_GUIS: [AtomicUsize; ENEMY_LIMIT] = [const { AtomicUsize::new(0) }; ENEMY_LIMIT];
static STATUS_GUI: AtomicUsize = AtomicUsize::new(0);
static ENEMY_STATUS_GUI: AtomicUsize = AtomicUsize::new(0);

/// Only Shipworks fights override campaign ally capacity and deployment delay.
pub(crate) fn test_allies() -> u32 {
    TEST_ALLIES.load(Ordering::Relaxed)
}

/// Install the two byte-checked native seams. Existing pre-combat input/click hooks service the UI.
///
/// # Safety
/// Serialized initialization of the verified Steam 1.163 image, before callbacks run.
pub(crate) unsafe fn install(session: &mut PatchSession, crash_button: bool) -> Result<(), String> {
    design::verify()?;
    for (address, expected) in [
        (FLEET_SEAM, &FLEET_BYTES[..]),
        (ENEMY_FLEET_SEAM, &ENEMY_FLEET_BYTES[..]),
        (CREATE_ESCADRA, &CREATE_ESCADRA_BYTES[..]),
        (ENEMY_APPEND_SEAM, &ENEMY_APPEND_BYTES[..]),
        (GET_PROFILE, &GET_PROFILE_BYTES[..]),
        (LEVEL_ENTRY, &LEVEL_BYTES[..]),
        (COPY_STRING, &COPY_STRING_BYTES[..]),
        (LINK_CLEAR_VECTOR, &LINK_CLEAR_VECTOR_BYTES[..]),
        (ENEMY_LOOP_SEAM, &ENEMY_LOOP_BYTES[..]),
    ] {
        if std::slice::from_raw_parts(address as *const u8, expected.len()) != expected {
            return Err(format!(
                "Shipworks allies: unexpected bytes at {address:#x}"
            ));
        }
    }
    let mut fleet = Trampoline::new();
    fleet.preserved_call(
        on_fleet as *const (),
        &[0x48, 0x89, 0xf9],
        &[],
        ReturnType::None,
    );
    fleet.bytes(&FLEET_BYTES);
    fleet.absolute_jump(FLEET_SEAM + FLEET_BYTES.len());
    session
        .detour_trampoline(FLEET_SEAM, FLEET_BYTES.len(), fleet)
        .map_err(|e| e.to_string())?;

    // Enemy Escadra creation: replace the CALL, forward the original arguments (the two
    // stack args are copied into the callback's slots by ENEMY_FLEET_SETUP), keep RAX.
    let mut enemy_fleet = Trampoline::new();
    enemy_fleet.preserved_call(
        on_enemy_fleet as *const (),
        &ENEMY_FLEET_SETUP,
        &[],
        ReturnType::Rax,
    );
    enemy_fleet.absolute_jump(ENEMY_FLEET_SEAM + ENEMY_FLEET_BYTES.len());
    session
        .detour_trampoline(ENEMY_FLEET_SEAM, ENEMY_FLEET_BYTES.len(), enemy_fleet)
        .map_err(|e| e.to_string())?;

    // Enemy Escadra is complete in RBX here; append the custom enemy snapshots before the
    // native LEA that starts linking the fleet lists. MOV RCX, RBX moves the escadra into
    // the callback's first argument slot; RCX is volatile, so preserved_call restores it.
    let mut enemy_append = Trampoline::new();
    enemy_append.preserved_call(
        on_enemy_append as *const (),
        &[0x48, 0x89, 0xd9], // MOV RCX, RBX
        &[],
        ReturnType::None,
    );
    enemy_append.bytes(&ENEMY_APPEND_BYTES);
    enemy_append.absolute_jump(ENEMY_APPEND_SEAM + ENEMY_APPEND_BYTES.len());
    session
        .detour_trampoline(ENEMY_APPEND_SEAM, ENEMY_APPEND_BYTES.len(), enemy_append)
        .map_err(|e| e.to_string())?;

    // Guard the Combat level loader's enemy-list walk against null escadra nodes (a previous
    // test battle's destroyed fleet leaves one behind): skip the node like the loop's own
    // advance instead of dereferencing [0+0x100].
    let mut enemy_loop = Trampoline::new();
    let loop_head = enemy_loop.new_label();
    let skip_node = enemy_loop.new_label();
    enemy_loop.bind(loop_head).map_err(|e| e.to_string())?;
    enemy_loop.bytes(&ENEMY_LOOP_BYTES[..4]); // MOV RAX, [R14+0x8]
    enemy_loop.bytes(&[0x48, 0x85, 0xc0]); // TEST RAX, RAX
    enemy_loop.jump_if(Condition::Equal, skip_node);
    enemy_loop.bytes(&ENEMY_LOOP_BYTES[4..]); // MOV RSI, [RAX+0x100]
    enemy_loop.absolute_jump(ENEMY_LOOP_BODY);
    enemy_loop.bind(skip_node).map_err(|e| e.to_string())?;
    enemy_loop.bytes(&[0x49, 0x83, 0xc6, 0x18]); // ADD R14, 0x18
    enemy_loop.bytes(&[0x4d, 0x3b, 0xf4]); // CMP R14, R12
    enemy_loop.jump_if(Condition::NotEqual, loop_head);
    enemy_loop.absolute_jump(ENEMY_LOOP_EXIT);
    session
        .detour_trampoline(ENEMY_LOOP_SEAM, ENEMY_LOOP_BYTES.len(), enemy_loop)
        .map_err(|e| e.to_string())?;

    let mut level = Trampoline::new();
    level.bytes(&LEVEL_BYTES);
    level.bytes(&[0x48, 0x83, 0xec, 0x08]); // Entry RSP is 8 mod 16.
    level.preserved_call(on_level as *const (), &[], &[], ReturnType::None);
    level.bytes(&[0x48, 0x83, 0xc4, 0x08]);
    level.absolute_jump(LEVEL_ENTRY + LEVEL_BYTES.len());
    session
        .detour_trampoline(LEVEL_ENTRY, LEVEL_BYTES.len(), level)
        .map_err(|e| e.to_string())?;
    CRASH_BUTTON.store(crash_button, Ordering::Relaxed);
    READY.store(true, Ordering::Relaxed);
    Ok(())
}

/// Read a bounded MSVC string as bytes, preserving the game's language encoding.
///
/// # Safety
/// The caller establishes the native object's identity/lifetime on the game thread.
/// Page checks detect malformed storage; they do not establish ownership.
unsafe fn native_string(address: usize) -> Option<CString> {
    let length = diag::rd::<usize>(address + 0x10)?;
    let capacity = diag::rd::<usize>(address + 0x18)?;
    if length > 256 || length > capacity {
        return None;
    }
    let data = if capacity > 15 {
        diag::rd::<usize>(address)?
    } else {
        address
    };
    if !diag::readable_span(data, length + 1) || read::<u8>(data + length) != 0 {
        return None;
    }
    let bytes = std::slice::from_raw_parts(data as *const u8, length);
    CString::new(bytes).ok()
}

/// Input-thread UI pass: only current, completely scanned GUI members may be reused or hidden.
pub(crate) unsafe fn tick() {
    if !READY.load(Ordering::Relaxed) {
        return;
    }
    if read::<u8>(EDITOR_MODE) == 0 && !CONTROLS_VISIBLE.load(Ordering::Relaxed) {
        return;
    }
    let scan = precombat::scan_gui_list();
    if !scan.complete {
        return;
    }
    let template = scan.elements.iter().copied().find(|&gui| {
        read::<i32>(gui + OFF_ACTIVE) == 2
            && precombat::cstr_is(gui + offset_of!(Gui, identifier), b"EDC_TEST")
    });
    let Some(template) = template else {
        hide_controls(&scan.elements);
        CONTROLS_VISIBLE.store(false, Ordering::Relaxed);
        return;
    };
    let Ok(state) = SESSION.try_lock() else {
        return;
    };
    let width = read::<f32>(template + 0x218);
    let height = read::<f32>(template + 0x21c);
    let x = read::<f32>(template + offset_of!(Body, position_x)) + width + BUTTON_GAP;
    let y = read::<f32>(template + offset_of!(Body, position_y));
    if ![x, y, width, height].iter().all(|v| v.is_finite()) || width <= 0.0 || height <= 0.0 {
        return;
    }
    show_control(
        ADD_NAME,
        &ADD_GUI,
        &scan.elements,
        template,
        c"ADD",
        ControlGeometry {
            position: [x, y],
            dimensions: [width, height],
        },
        BUTTON_STYLE,
    );
    show_control(
        ENEMY_ADD_NAME,
        &ENEMY_ADD_GUI,
        &scan.elements,
        template,
        c"ADD ENEMY",
        ControlGeometry {
            position: [x + width + BUTTON_GAP, y],
            dimensions: [width, height],
        },
        BUTTON_STYLE,
    );
    show_control(
        CLEAR_NAME,
        &CLEAR_GUI,
        &scan.elements,
        template,
        c"CLEAR",
        ControlGeometry {
            position: [x + 2.0 * (width + BUTTON_GAP), y],
            dimensions: [width, height],
        },
        BUTTON_STYLE,
    );
    if CRASH_BUTTON.load(Ordering::Relaxed) {
        show_control(
            CRASH_NAME,
            &CRASH_GUI,
            &scan.elements,
            template,
            c"CRASH",
            ControlGeometry {
                position: [x + 3.0 * (width + BUTTON_GAP), y],
                dimensions: [width, height],
            },
            BUTTON_STYLE,
        );
    }
    let left = x - width * 0.5;
    let list_top = y + height * 0.5 + 24.0;
    for (index, entry) in state.entries.iter().enumerate() {
        let position = [left, list_top + index as f32 * ROW_HEIGHT];
        show_control(
            ROW_NAMES[index],
            &ROW_GUIS[index],
            &scan.elements,
            template,
            &entry.name,
            ControlGeometry {
                position,
                dimensions: [LIST_WIDTH, ROW_HEIGHT],
            },
            ROW_STYLE,
        );
    }
    for gui in &ROW_GUIS[state.entries.len()..] {
        hide_member(gui.load(Ordering::Relaxed), &scan.elements);
    }
    let mut next_row = state.entries.len();
    if let Some(status) = &state.status {
        let position = [left, list_top + next_row as f32 * ROW_HEIGHT];
        show_control(
            STATUS_NAME,
            &STATUS_GUI,
            &scan.elements,
            template,
            status,
            ControlGeometry {
                position,
                dimensions: [LIST_WIDTH, ROW_HEIGHT],
            },
            ROW_STYLE,
        );
        next_row += 1;
    } else {
        hide_member(STATUS_GUI.load(Ordering::Relaxed), &scan.elements);
    }
    for (index, entry) in state.enemy_entries.iter().enumerate() {
        let position = [left, list_top + (next_row + index) as f32 * ROW_HEIGHT];
        show_control(
            ENEMY_ROW_NAMES[index],
            &ENEMY_ROW_GUIS[index],
            &scan.elements,
            template,
            &entry.name,
            ControlGeometry {
                position,
                dimensions: [LIST_WIDTH, ROW_HEIGHT],
            },
            ENEMY_ROW_STYLE,
        );
    }
    for gui in &ENEMY_ROW_GUIS[state.enemy_entries.len()..] {
        hide_member(gui.load(Ordering::Relaxed), &scan.elements);
    }
    next_row += state.enemy_entries.len();
    if let Some(status) = &state.enemy_status {
        let position = [left, list_top + next_row as f32 * ROW_HEIGHT];
        show_control(
            ENEMY_STATUS_NAME,
            &ENEMY_STATUS_GUI,
            &scan.elements,
            template,
            status,
            ControlGeometry {
                position,
                dimensions: [LIST_WIDTH, ROW_HEIGHT],
            },
            ENEMY_ROW_STYLE,
        );
    } else {
        hide_member(ENEMY_STATUS_GUI.load(Ordering::Relaxed), &scan.elements);
    }
    CONTROLS_VISIBLE.store(true, Ordering::Relaxed);
}

/// Initialize once, with the top-row TEST control's actual native font/backgrounds.
/// Labels use the same flash_large_shadow font as the mod's PLAYER marker.
unsafe fn show_control(
    name: &[u8],
    kept: &AtomicUsize,
    listed: &[usize],
    template: usize,
    text: &CStr,
    geometry: ControlGeometry,
    style: ControlStyle,
) {
    let create = || create_control(name, template, text, geometry, style);
    let Some((gui, created)) = precombat::get_or_make_with(name, kept, listed, true, create) else {
        return;
    };
    precombat::set_gui_active(gui, true);
    if !created {
        precombat::move_gui_rect(gui, geometry.position, geometry.dimensions);
        // Native std::string assignment preserves allocator ownership and any existing capacity.
        let assign: unsafe extern "C" fn(usize, *const i8, usize) -> usize =
            std::mem::transmute(COPY_STRING);
        assign(
            gui + offset_of!(Gui, text),
            text.as_ptr(),
            text.to_bytes().len(),
        );
        if let Some(color) = style.color {
            write(gui + offset_of!(Gui, text_color), color);
        }
        if !CONTROLS_VISIBLE.load(Ordering::Relaxed) {
            precombat::load_gui(gui);
        }
    }
}

unsafe fn create_control(
    name: &[u8],
    template: usize,
    text: &CStr,
    geometry: ControlGeometry,
    style: ControlStyle,
) -> *mut u8 {
    let empty = c"";
    let backgrounds = [
        offset_of!(Gui, backname),
        offset_of!(Gui, backname_focus),
        offset_of!(Gui, backname_use),
    ]
    .map(|offset| {
        if style.button {
            native_string(template + offset)
        } else {
            Some(empty.into())
        }
    });
    let font = if style.button {
        native_string(template + offset_of!(Gui, font))
    } else {
        Some(c"flash_large_shadow".into())
    };
    let (Some(back), Some(focus), Some(pressed), Some(font)) =
        (&backgrounds[0], &backgrounds[1], &backgrounds[2], font)
    else {
        return std::ptr::null_mut();
    };
    let pointer = precombat::allocate_gui();
    if pointer.is_null() {
        return pointer;
    }
    let gui = pointer as usize;
    let vtable: usize = read(gui);
    let init: precombat::GuiInitFn = std::mem::transmute(read::<usize>(vtable + 0x2c0));
    // SAFETY: allocate_gui constructed this GUI with the native allocator/constructor.
    // init copies each borrowed string; the game owns all resulting strings, geometry and resources.
    init(
        gui as *mut u8,
        name.as_ptr(),
        geometry.position[0],
        geometry.position[1],
        geometry.dimensions[0],
        geometry.dimensions[1],
        text.as_ptr().cast(),
        back.as_ptr().cast(),
        focus.as_ptr().cast(),
        pressed.as_ptr().cast(),
        font.as_ptr().cast(),
        if style.button {
            read(template + offset_of!(Gui, font_scale))
        } else {
            1.0
        },
    );
    write(
        gui + offset_of!(Body, stage),
        read::<i32>(template + offset_of!(Body, stage)),
    );
    write(
        gui + offset_of!(Body, layer),
        read::<i32>(template + offset_of!(Body, layer)),
    );
    write(gui + offset_of!(Body, focusable), u8::from(style.button));
    write(gui + offset_of!(Gui, clickable), u8::from(style.button));
    write(gui + offset_of!(Gui, checkable), 0u8);
    write(gui + offset_of!(Gui, textable), 0u8);
    write(
        gui + offset_of!(Gui, text_color),
        style
            .color
            .unwrap_or_else(|| read::<u32>(template + offset_of!(Gui, text_color))),
    );
    write(
        gui + offset_of!(Gui, align),
        if style.button {
            read::<i32>(template + offset_of!(Gui, align))
        } else {
            0
        },
    );
    precombat::attach_gui(pointer);
    precombat::load_gui(gui);
    pointer
}

unsafe fn write<T>(address: usize, value: T) {
    (address as *mut T).write_unaligned(value);
}

unsafe fn hide_member(gui: usize, listed: &[usize]) {
    if listed.contains(&gui)
        && native_string(gui + offset_of!(Gui, identifier)).is_some_and(|name| {
            let bytes = name.to_bytes();
            bytes.starts_with(b"SW_ALLY_") || bytes.starts_with(b"SW_ENEMY_")
        })
    {
        precombat::set_gui_active(gui, false);
    }
}

unsafe fn hide_controls(listed: &[usize]) {
    hide_member(ADD_GUI.load(Ordering::Relaxed), listed);
    hide_member(ENEMY_ADD_GUI.load(Ordering::Relaxed), listed);
    hide_member(CLEAR_GUI.load(Ordering::Relaxed), listed);
    hide_member(CRASH_GUI.load(Ordering::Relaxed), listed);
    hide_member(STATUS_GUI.load(Ordering::Relaxed), listed);
    hide_member(ENEMY_STATUS_GUI.load(Ordering::Relaxed), listed);
    for gui in &ROW_GUIS {
        hide_member(gui.load(Ordering::Relaxed), listed);
    }
    for gui in &ENEMY_ROW_GUIS {
        hide_member(gui.load(Ordering::Relaxed), listed);
    }
}

/// Called from the existing native click dispatcher, on the game thread.
pub(crate) unsafe fn on_click(name: &str, button: usize) -> bool {
    if !matches!(
        name,
        "SW_ALLY_ADD" | "SW_ENEMY_ADD" | "SW_ALLY_CLEAR" | "SW_ALLY_CRASH"
    ) || !READY.load(Ordering::Relaxed)
    {
        return false;
    }
    if button as u32 != 1 {
        return true;
    }
    if read::<u8>(EDITOR_MODE) == 0 || !CONTROLS_VISIBLE.load(Ordering::Relaxed) {
        return true;
    }
    if name == "SW_ALLY_CRASH" {
        if CRASH_BUTTON.load(Ordering::Relaxed) {
            log::error!("hf_ally: deliberate Shipworks fail-fast requested");
            // SAFETY: Windows accepts null records and generates STATUS_FAIL_FAST_EXCEPTION.
            // Fail-fast terminates without deliberate invalid memory access, save writes or
            // running native teardown on partially modified state. It bypasses exception handlers;
            // this is not a test of our ordinary access-violation/minidump handler.
            windows_sys::Win32::System::Diagnostics::Debug::RaiseFailFastException(
                std::ptr::null(),
                std::ptr::null(),
                0,
            );
        }
        return true;
    }
    let Ok(mut state) = SESSION.try_lock() else {
        return true;
    };
    let now = now_ms();
    let clear = name == "SW_ALLY_CLEAR";
    let enemy_add = name == "SW_ENEMY_ADD";
    let last_click_ms = if clear {
        &mut state.last_clear_ms
    } else if enemy_add {
        &mut state.last_enemy_add_ms
    } else {
        &mut state.last_add_ms
    };
    if last_click_ms.is_some_and(|last| now.wrapping_sub(last) <= CLICK_DEBOUNCE_MS) {
        return true;
    }
    *last_click_ms = Some(now);
    if clear {
        let allies = state.entries.len();
        let enemies = state.enemy_entries.len();
        clear_roster(&mut state);
        drop(state);
        // Also drop the last test enemy Escadra from the profile so a later FIGHT (or a
        // campaign battle) does not re-link the cleared enemies.
        dump_enemy_state("clear");
        prune_enemy_escadra();
        sound::button("Shipworks CLEAR");
        log::info!("hf_ally: Shipworks CLEAR removed {allies} allies and {enemies} enemies");
        return true;
    }
    let limit = if enemy_add { ENEMY_LIMIT } else { ROSTER_LIMIT };
    let len = if enemy_add {
        state.enemy_entries.len()
    } else {
        state.entries.len()
    };
    if len == limit {
        if enemy_add {
            state.enemy_status = Some(c"ENEMY LIST FULL (8 ENEMIES)".into());
        } else {
            state.status = Some(c"SHIP LIST FULL (8 ALLIES)".into());
        }
        return true;
    }
    state.serial = state.serial.wrapping_add(1);
    let serial = state.serial;
    match design::snapshot(serial) {
        Ok(entry) => {
            log::info!(
                "hf_ally: Shipworks {} {}",
                if enemy_add { "ADD ENEMY" } else { "ADD" },
                entry.name.to_string_lossy()
            );
            if enemy_add {
                state.enemy_entries.push(entry);
                state.enemy_status = None;
            } else {
                state.entries.push(entry);
                state.status = None;
            }
            sound::button(if enemy_add {
                "Shipworks ADD ENEMY"
            } else {
                "Shipworks ADD"
            });
        }
        Err(error) => {
            if enemy_add {
                state.enemy_status = Some(c"UNABLE TO ADD THIS SHIP".into());
            } else {
                state.status = Some(c"UNABLE TO ADD THIS SHIP".into());
            }
            log::warn!(
                "hf_ally: Shipworks {} failed: {error}; roster unchanged",
                if enemy_add { "ADD ENEMY" } else { "ADD" }
            );
        }
    }
    true
}

unsafe extern "C" fn on_fleet(fleet: usize) {
    let _hook = diag::scope("Shipworks test fleet");
    let Ok(mut state) = SESSION.try_lock() else {
        return;
    };
    let mut added = 0;
    for entry in &state.entries {
        if design::append(fleet, &entry.path, SIDE_ALLY) {
            added += 1;
        } else {
            log::warn!(
                "hf_ally: failed to load test ally {}",
                entry.name.to_string_lossy()
            );
        }
    }
    if added > 0 {
        let mask = ((1u32 << added) - 1) << 2;
        let previous = precombat::replace_selected_mask(mask);
        state.saved_mask.get_or_insert(previous);
    }
    TEST_ALLIES.store(added, Ordering::Relaxed);
    log::info!("hf_ally: Shipworks FIGHT added {added} allies");
}

type CreateEscadra = unsafe extern "C" fn(usize, f32, u32, usize, usize, u32) -> usize;

/// One-line bounded dump of where enemy escadras currently live: the stored test escadra, the
/// profile's escadra vector (8-byte cells) and the enemy-side fleet list (0x18-byte nodes).
/// Pointer values are only compared, never dereferenced beyond page-checked scalar reads.
///
/// # Safety
/// Game thread only. All reads go through `diag::rd` (page-checked) and are count-bounded.
unsafe fn dump_enemy_state(tag: &str) {
    let stored = SESSION
        .try_lock()
        .map(|state| state.enemy_escadra)
        .unwrap_or(0);
    let get_profile: unsafe extern "C" fn() -> usize = std::mem::transmute(GET_PROFILE);
    let profile = get_profile();
    let (pbegin, pend) = (
        diag::rd::<usize>(profile.wrapping_add(PROFILE_ESCADRAS)).unwrap_or(0),
        diag::rd::<usize>(profile.wrapping_add(PROFILE_ESCADRAS + 8)).unwrap_or(0),
    );
    let mut profile_entries = String::new();
    let pcount = if pbegin != 0 && pend >= pbegin && (pend - pbegin) / 8 <= LIST_ENTRY_LIMIT {
        for index in 0..(pend - pbegin) / 8 {
            let fleet = diag::rd::<usize>(pbegin + index * 8).unwrap_or(0);
            let ships = diag::rd::<usize>(fleet.wrapping_add(ESCADRA_SHIP_COUNT)).unwrap_or(0);
            let _ = write!(
                profile_entries,
                " {fleet:#x}({}{ships})",
                if fleet == stored { "*" } else { "" },
            );
        }
        (pend - pbegin) / 8
    } else {
        0
    };
    let (lbegin, lend) = (
        diag::rd::<usize>(ENEMY_LIST).unwrap_or(0),
        diag::rd::<usize>(ENEMY_LIST + 8).unwrap_or(0),
    );
    let mut list_entries = String::new();
    let lcount = if lbegin != 0
        && lend >= lbegin
        && (lend - lbegin) / LINK_NODE_STRIDE <= LIST_ENTRY_LIMIT
    {
        for index in 0..(lend - lbegin) / LINK_NODE_STRIDE {
            let fleet = diag::rd::<usize>(lbegin + index * LINK_NODE_STRIDE + 8).unwrap_or(0);
            let ships = diag::rd::<usize>(fleet.wrapping_add(ESCADRA_SHIP_COUNT)).unwrap_or(0);
            let _ = write!(
                list_entries,
                " {fleet:#x}({}{ships})",
                if fleet == stored { "*" } else { "" },
            );
        }
        (lend - lbegin) / LINK_NODE_STRIDE
    } else {
        0
    };
    log::info!(
        "hf_ally: enemy state {tag}: stored={stored:#x} profile[{pcount}]{profile_entries} list[{lcount}]{list_entries}"
    );
}

/// Remove the previously created test enemy Escadra from both containers that can re-link it
/// into a later battle: the profile's escadra vector (8-byte cells at profile+0x1f8) and the
/// enemy-side fleet list (0x18-byte link nodes at 0x143a159c0, escadra at node+0x8). Only
/// pointer values are compared; the escadra object is never dereferenced, so a game-destroyed
/// object cannot be read through a stale pointer.
///
/// # Safety
/// Game thread only (on_enemy_fleet / on_click / on_level callbacks). Both vectors are
/// bounds-checked before any scan or write.
unsafe fn prune_enemy_escadra() {
    let escadra = match SESSION.try_lock() {
        Ok(state) => state.enemy_escadra,
        Err(_) => return,
    };
    if escadra == 0 {
        return;
    }
    if let Ok(mut state) = SESSION.try_lock() {
        state.enemy_escadra = 0;
    }
    let get_profile: unsafe extern "C" fn() -> usize = std::mem::transmute(GET_PROFILE);
    let profile = get_profile();
    let (pbegin, pend) = (
        diag::rd::<usize>(profile.wrapping_add(PROFILE_ESCADRAS)).unwrap_or(0),
        diag::rd::<usize>(profile.wrapping_add(PROFILE_ESCADRAS + 8)).unwrap_or(0),
    );
    let pbytes = pend.wrapping_sub(pbegin);
    if pbegin != 0
        && pend >= pbegin
        && pbytes.is_multiple_of(8)
        && pbytes <= ESCADRA_VECTOR_LIMIT
        && diag::readable_span(pbegin, pbytes)
    {
        let mut cell = pbegin;
        while cell < pend {
            if read::<usize>(cell) == escadra {
                let mut source = cell + 8;
                while source < pend {
                    write::<usize>(source - 8, read::<usize>(source));
                    source += 8;
                }
                write::<usize>(profile + PROFILE_ESCADRAS + 8, pend - 8);
                log::info!("hf_ally: pruned previous test enemy escadra {escadra:#x} from the profile escadra vector");
                return;
            }
            cell += 8;
        }
    }
    let (lbegin, lend) = (
        diag::rd::<usize>(ENEMY_LIST).unwrap_or(0),
        diag::rd::<usize>(ENEMY_LIST + 8).unwrap_or(0),
    );
    let lbytes = lend.wrapping_sub(lbegin);
    if lbegin != 0
        && lend >= lbegin
        && lbytes.is_multiple_of(LINK_NODE_STRIDE)
        && lbytes / LINK_NODE_STRIDE <= LIST_ENTRY_LIMIT
        && diag::readable_span(lbegin, lbytes)
    {
        // Compact the list, dropping the stored test escadra and any null-escadra nodes left
        // behind by a destroyed fleet (the level loader dereferences node+8 without a check).
        let mut kept = lbegin;
        let mut node = lbegin;
        let mut dropped_stored = false;
        let mut dropped_null = 0usize;
        while node < lend {
            let fleet = read::<usize>(node + 8);
            if fleet == escadra {
                dropped_stored = true;
            } else if fleet == 0 {
                dropped_null += 1;
            } else {
                if kept != node {
                    for offset in 0..LINK_NODE_STRIDE {
                        write::<u8>(kept + offset, read::<u8>(node + offset));
                    }
                }
                kept += LINK_NODE_STRIDE;
            }
            node += LINK_NODE_STRIDE;
        }
        if kept != lend {
            write::<usize>(ENEMY_LIST + 8, kept);
        }
        if dropped_stored {
            log::info!(
                "hf_ally: pruned previous test enemy escadra {escadra:#x} from the enemy fleet list"
            );
            return;
        }
        if dropped_null > 0 {
            log::info!(
                "hf_ally: dropped {dropped_null} null-escadra nodes from the enemy fleet list"
            );
            return;
        }
    }
    log::info!(
        "hf_ally: previous test enemy escadra {escadra:#x} not found in profile vector or enemy list"
    );
}

/// Replaces the FIGHT call that builds the enemy Escadra from generated ship names.
/// With a custom enemy roster, the Escadra is created from a single unresolvable name, so
/// the default generation adds no ships; the roster's snapshots are appended at
/// ENEMY_APPEND_SEAM. Without one, the original call runs unchanged.
///
/// # Safety
/// Runs on the game thread inside the byte-checked FIGHT seam; arguments are the original
/// Fleet_CreateEscadraFromShips slots (names vector, side/price, role, position, name,
/// morale) forwarded by ENEMY_FLEET_SETUP.
unsafe extern "C" fn on_enemy_fleet(
    names: usize,
    side: f32,
    role: u32,
    position: usize,
    name: usize,
    morale: u32,
) -> usize {
    let _hook = diag::scope("Shipworks test enemy fleet");
    let create: CreateEscadra = std::mem::transmute(CREATE_ESCADRA);
    dump_enemy_state("fight entry");
    // Drop the previous test enemy Escadra from the profile before this battle: the Combat
    // level loader re-links every profile escadra into the enemy fleet, so leaving it would
    // accumulate the previous fight's enemies into this one.
    prune_enemy_escadra();
    let custom = SESSION
        .try_lock()
        .map(|state| !state.enemy_entries.is_empty())
        .unwrap_or(false);
    if custom {
        // The native FIGHT handler clears the enemy list right after this seam and links the
        // new escadra, but a node left by a destroyed previous fleet can still crash the level
        // loader; clear the list here as well so only the fresh escadra is ever linked.
        let clear: unsafe extern "C" fn(usize) = std::mem::transmute(LINK_CLEAR_VECTOR);
        // SAFETY: byte-checked native Link_ClearVector on the enemy list global; it unlinks
        // each node (null-safe) and resets end=begin on the game thread.
        clear(ENEMY_LIST);
    }
    if !custom {
        // SAFETY: original arguments, original callee, game thread.
        return create(names, side, role, position, name, morale);
    }
    // One 0x20-byte inline (SSO) EscadraName that no library design resolves to; the native
    // renderShipPhoto__ lookup returns null for it and the ship is skipped.
    let bogus_name = b"HF_NO_SUCH_DESIGN\0";
    let mut element = [0u8; 0x20];
    element[..bogus_name.len()].copy_from_slice(bogus_name);
    element[0x10..0x18].copy_from_slice(&((bogus_name.len() - 1) as u64).to_le_bytes());
    element[0x18..0x20].copy_from_slice(&15u64.to_le_bytes());
    let mut vector = [
        element.as_ptr() as usize,
        element.as_ptr() as usize + 0x20,
        element.as_ptr() as usize + 0x20,
    ];
    // SAFETY: vector outlives the synchronous native call; the escadra and every game-side
    // allocation remain natively owned.
    let escadra = create(
        vector.as_mut_ptr() as usize,
        SIDE_ENEMY,
        role,
        position,
        name,
        morale,
    );
    if escadra != 0 {
        if let Ok(mut state) = SESSION.try_lock() {
            state.enemy_escadra = escadra;
        }
    }
    log::info!("hf_ally: Shipworks FIGHT overrides default enemy generation");
    dump_enemy_state("fight built");
    escadra
}

/// Appends the custom enemy snapshots to the freshly created enemy Escadra (RDI = RBX).
///
/// # Safety
/// Runs on the game thread at the byte-checked ENEMY_APPEND_SEAM, after the Escadra's native
/// init, before the fleet lists are linked.
unsafe extern "C" fn on_enemy_append(escadra: usize) {
    let _hook = diag::scope("Shipworks test enemies");
    let Ok(state) = SESSION.try_lock() else {
        return;
    };
    let mut added = 0;
    for entry in &state.enemy_entries {
        if design::append(escadra, &entry.path, SIDE_ENEMY) {
            added += 1;
        } else {
            log::warn!(
                "hf_ally: failed to load test enemy {}",
                entry.name.to_string_lossy()
            );
        }
    }
    log::info!("hf_ally: Shipworks FIGHT added {added} enemies");
    dump_enemy_state("fight appended");
}

unsafe extern "C" fn on_level(path: usize) {
    let _hook = diag::scope("Shipworks level");
    let mut bytes = Vec::new();
    for index in 0..64 {
        let Some(byte) = diag::rd::<u8>(path + index) else {
            return;
        };
        if byte == 0 {
            break;
        }
        bytes.push(byte);
    }
    // A test/landing trip and its return are still the same Shipworks session.
    if bytes == b"Combat.seria"
        || (bytes == b"City_generated.seria" && read::<u8>(TEST_FIGHT_FLAG) != 0)
    {
        return;
    }
    let Ok(mut state) = SESSION.try_lock() else {
        return;
    };
    TEST_ALLIES.store(0, Ordering::Relaxed);
    if let Some(mask) = state.saved_mask.take() {
        precombat::replace_selected_mask(mask);
    }
    if bytes == b"Editor.seria" {
        return;
    }
    clear_roster(&mut state);
    state.last_clear_ms = None;
    drop(state);
    dump_enemy_state("leaving shipworks");
    prune_enemy_escadra();
}

/// Drop both rosters' snapshots and statuses; keep the editor ship and capture serial intact.
/// Called for both explicit CLEAR and leaving Shipworks.
fn clear_roster(state: &mut Session) {
    for entry in state.entries.drain(..).chain(state.enemy_entries.drain(..)) {
        if let Ok(path) = entry.path.to_str() {
            if let Err(error) = std::fs::remove_file(path) {
                log::warn!("hf_ally: cannot remove test snapshot {path}: {error}");
            }
        }
    }
    state.status = None;
    state.enemy_status = None;
    state.last_add_ms = None;
    state.last_enemy_add_ms = None;
}
