//! Session-owned Shipworks test roster. Native designs are serialized snapshots, never editor
//! pointers. The existing FIGHT builder transfers fresh native copies into its player fleet.
//! Native evidence and manual validation checklist: docs/shipworks-test-allies.md.

mod design;

use std::{
    ffi::{CStr, CString},
    mem::offset_of,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        Mutex,
    },
};

use patchy::{PatchSession, ReturnType, Trampoline};

use crate::{
    diag,
    native::{read, OFF_ACTIVE},
    native_layout::{Body, Gui},
    precombat, sound,
    time::now_ms,
};

/// Nine native player cards: the editor ship plus eight allies.
const ROSTER_LIMIT: usize = 8;
const ADD_NAME: &[u8] = b"SW_ALLY_ADD\0";
const CLEAR_NAME: &[u8] = b"SW_ALLY_CLEAR\0";
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
const STATUS_NAME: &[u8] = b"SW_ALLY_STATUS\0";
const BUTTON_GAP: f32 = 12.0;
const ROW_HEIGHT: f32 = 24.0;
const LIST_WIDTH: f32 = 480.0;
const CLICK_DEBOUNCE_MS: u64 = 120;
const EDITOR_MODE: usize = 0x1_403e_f36a;
const TEST_FIGHT_FLAG: usize = 0x1_47fc_6fb6;
/// FIGHT: first ship is already attached to RDI; replay MOV RSI,[RDI+0x100] after adding ours.
const FLEET_SEAM: usize = 0x1_4013_dcc2;
const FLEET_BYTES: [u8; 7] = [0x48, 0x8b, 0xb7, 0x00, 0x01, 0x00, 0x00];
const LEVEL_ENTRY: usize = 0x1_4025_afd0;
const LEVEL_BYTES: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18];
const COPY_STRING: usize = 0x1_4000_56e0;
const COPY_STRING_BYTES: [u8; 12] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x57,
];

struct Entry {
    name: CString,
    path: CString,
}

#[derive(Clone, Copy)]
struct ControlGeometry {
    position: [f32; 2],
    dimensions: [f32; 2],
}

struct Session {
    entries: Vec<Entry>,
    status: Option<CString>,
    serial: u64,
    last_add_ms: Option<u64>,
    last_clear_ms: Option<u64>,
    saved_mask: Option<u32>,
}

static SESSION: Mutex<Session> = Mutex::new(Session {
    entries: Vec::new(),
    status: None,
    serial: 0,
    last_add_ms: None,
    last_clear_ms: None,
    saved_mask: None,
});
static READY: AtomicBool = AtomicBool::new(false);
static CONTROLS_VISIBLE: AtomicBool = AtomicBool::new(false);
static TEST_ALLIES: AtomicU32 = AtomicU32::new(0);
static ADD_GUI: AtomicUsize = AtomicUsize::new(0);
static CLEAR_GUI: AtomicUsize = AtomicUsize::new(0);
static ROW_GUIS: [AtomicUsize; ROSTER_LIMIT] = [const { AtomicUsize::new(0) }; ROSTER_LIMIT];
static STATUS_GUI: AtomicUsize = AtomicUsize::new(0);

/// Only Shipworks fights override campaign ally capacity and deployment delay.
pub(crate) fn test_allies() -> u32 {
    TEST_ALLIES.load(Ordering::Relaxed)
}

/// Install the two byte-checked native seams. Existing pre-combat input/click hooks service the UI.
///
/// # Safety
/// Serialized initialization of the verified Steam 1.163 image, before callbacks run.
pub(crate) unsafe fn install(session: &mut PatchSession) -> Result<(), String> {
    design::verify()?;
    for (address, expected) in [
        (FLEET_SEAM, &FLEET_BYTES[..]),
        (LEVEL_ENTRY, &LEVEL_BYTES[..]),
        (COPY_STRING, &COPY_STRING_BYTES[..]),
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

    let mut level = Trampoline::new();
    level.bytes(&LEVEL_BYTES);
    level.bytes(&[0x48, 0x83, 0xec, 0x08]); // Entry RSP is 8 mod 16.
    level.preserved_call(on_level as *const (), &[], &[], ReturnType::None);
    level.bytes(&[0x48, 0x83, 0xc4, 0x08]);
    level.absolute_jump(LEVEL_ENTRY + LEVEL_BYTES.len());
    session
        .detour_trampoline(LEVEL_ENTRY, LEVEL_BYTES.len(), level)
        .map_err(|e| e.to_string())?;
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
        true,
    );
    show_control(
        CLEAR_NAME,
        &CLEAR_GUI,
        &scan.elements,
        template,
        c"CLEAR",
        ControlGeometry {
            position: [x + width + BUTTON_GAP, y],
            dimensions: [width, height],
        },
        true,
    );
    let left = x - width * 0.5;
    for (index, entry) in state.entries.iter().enumerate() {
        let position = [left, y + height * 0.5 + 24.0 + index as f32 * ROW_HEIGHT];
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
            false,
        );
    }
    for gui in &ROW_GUIS[state.entries.len()..] {
        hide_member(gui.load(Ordering::Relaxed), &scan.elements);
    }
    if let Some(status) = &state.status {
        let position = [
            left,
            y + height * 0.5 + 24.0 + state.entries.len() as f32 * ROW_HEIGHT,
        ];
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
            false,
        );
    } else {
        hide_member(STATUS_GUI.load(Ordering::Relaxed), &scan.elements);
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
    button: bool,
) {
    let create = || create_control(name, template, text, geometry, button);
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
    button: bool,
) -> *mut u8 {
    let empty = c"";
    let backgrounds = [
        offset_of!(Gui, backname),
        offset_of!(Gui, backname_focus),
        offset_of!(Gui, backname_use),
    ]
    .map(|offset| {
        if button {
            native_string(template + offset)
        } else {
            Some(empty.into())
        }
    });
    let font = if button {
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
        if button {
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
    write(gui + offset_of!(Body, focusable), u8::from(button));
    write(gui + offset_of!(Gui, clickable), u8::from(button));
    write(gui + offset_of!(Gui, checkable), 0u8);
    write(gui + offset_of!(Gui, textable), 0u8);
    write(
        gui + offset_of!(Gui, text_color),
        read::<u32>(template + offset_of!(Gui, text_color)),
    );
    write(
        gui + offset_of!(Gui, align),
        if button {
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
        && native_string(gui + offset_of!(Gui, identifier))
            .is_some_and(|name| name.to_bytes().starts_with(b"SW_ALLY_"))
    {
        precombat::set_gui_active(gui, false);
    }
}

unsafe fn hide_controls(listed: &[usize]) {
    hide_member(ADD_GUI.load(Ordering::Relaxed), listed);
    hide_member(CLEAR_GUI.load(Ordering::Relaxed), listed);
    hide_member(STATUS_GUI.load(Ordering::Relaxed), listed);
    for gui in &ROW_GUIS {
        hide_member(gui.load(Ordering::Relaxed), listed);
    }
}

/// Called from the existing native click dispatcher, on the game thread.
pub(crate) unsafe fn on_click(name: &str, button: usize) -> bool {
    let clear = match name {
        "SW_ALLY_ADD" => false,
        "SW_ALLY_CLEAR" => true,
        _ => return false,
    };
    if !READY.load(Ordering::Relaxed) {
        return false;
    }
    if button as u32 != 1 {
        return true;
    }
    if read::<u8>(EDITOR_MODE) == 0 || !CONTROLS_VISIBLE.load(Ordering::Relaxed) {
        return true;
    }
    let Ok(mut state) = SESSION.try_lock() else {
        return true;
    };
    let now = now_ms();
    let last_click_ms = if clear {
        &mut state.last_clear_ms
    } else {
        &mut state.last_add_ms
    };
    if last_click_ms.is_some_and(|last| now.wrapping_sub(last) <= CLICK_DEBOUNCE_MS) {
        return true;
    }
    *last_click_ms = Some(now);
    if clear {
        let count = state.entries.len();
        clear_roster(&mut state);
        sound::button("Shipworks CLEAR");
        log::info!("hf_ally: Shipworks CLEAR removed {count} allies");
        return true;
    }
    if state.entries.len() == ROSTER_LIMIT {
        state.status = Some(c"SHIP LIST FULL (8 ALLIES)".into());
        return true;
    }
    state.serial = state.serial.wrapping_add(1);
    match design::snapshot(state.serial) {
        Ok(entry) => {
            log::info!("hf_ally: Shipworks ADD {}", entry.name.to_string_lossy());
            state.entries.push(entry);
            state.status = None;
            sound::button("Shipworks ADD");
        }
        Err(error) => {
            state.status = Some(c"UNABLE TO ADD THIS SHIP".into());
            log::warn!("hf_ally: Shipworks ADD failed: {error}; roster unchanged");
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
        if design::append(fleet, &entry.path) {
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
}

/// Drop only this roster's snapshots and status; keep the editor ship and capture serial intact.
/// Called for both explicit CLEAR and leaving Shipworks.
fn clear_roster(state: &mut Session) {
    for entry in state.entries.drain(..) {
        if let Ok(path) = entry.path.to_str() {
            if let Err(error) = std::fs::remove_file(path) {
                log::warn!("hf_ally: cannot remove test snapshot {path}: {error}");
            }
        }
    }
    state.status = None;
    state.last_add_ms = None;
}
