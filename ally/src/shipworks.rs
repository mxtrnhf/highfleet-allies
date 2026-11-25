//! Session-owned Shipworks test roster. Native designs are serialized snapshots, never editor
//! pointers. The existing FIGHT builder transfers fresh native copies into its player fleet.
//! Native evidence and manual validation checklist: docs/shipworks-test-allies.md.

mod airstrike;
mod damage;
mod design;
mod missile_defense;
mod silent_strike;
mod strategic_blast;

use std::{
    ffi::{CStr, CString},
    fmt::Write as _,
    mem::{offset_of, size_of},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
};

use patchy::{Condition, PatchSession, ReturnType, Trampoline};

use crate::{
    diag,
    native::{read, OFF_ACTIVE},
    native_layout::{Body, Gui, Link},
    precombat, sound,
    time::now_ms,
};

/// Nine native player cards: the editor ship plus eight allies.
const ROSTER_LIMIT: usize = 8;
/// Enemy test roster mirrors the ally limit; both lists render in the same column.
const ENEMY_LIMIT: usize = 8;
/// GUI identifiers must stay within the 15-char inline (SSO) string storage: `cstr` reads the
/// identifier bytes in place, so a longer heap-allocated name would never compare equal.
const PANEL_TOGGLE_NAME: &CStr = c"SW_ALLY_TOGGLE";
const ADD_NAME: &[u8] = b"SW_ALLY_ADD\0";
const ENEMY_ADD_NAME: &[u8] = b"SW_ENEMY_ADD\0";
const CLEAR_NAME: &[u8] = b"SW_ALLY_CLEAR\0";
const ACE_NAME: &CStr = c"SW_ACE";
const ACE_DEFENSE_NAME: &CStr = c"SW_ACE_DEFENSE";
const CRASH_NAME: &[u8] = b"SW_ALLY_CRASH\0";
const ACE_OPTIONS: [(&CStr, &CStr); 3] = [
    (airstrike::BUTTON, c"AIRSTRIKE"),
    (ACE_DEFENSE_NAME, c"MISSILE DEFENSE"),
    (silent_strike::BUTTON, c"SILENT STRIKE"),
];
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
const ACE_OPTION_GAP: f32 = 4.0;
const ROW_HEIGHT: f32 = 24.0;
const LIST_WIDTH: f32 = 480.0;
/// Gap between the button row and the first row of the lists (GUI units).
const LIST_TOP_GAP: f32 = 24.0;
const CLICK_DEBOUNCE_MS: u64 = 120;
/// ARGB; red-dish hue marks the enemy list against the ally list's native colour.
const ENEMY_TEXT_COLOR: u32 = 0xffff_5050;
const BUTTON_STYLE: ControlStyle = ControlStyle {
    button: true,
    clickable: true,
    color: None,
};
const ROW_STYLE: ControlStyle = ControlStyle {
    button: false,
    clickable: false,
    color: None,
};
const ENEMY_ROW_STYLE: ControlStyle = ControlStyle {
    button: false,
    clickable: false,
    color: Some(ENEMY_TEXT_COLOR),
};
/// Creature side (+0xad4): positive is the player side, negative is the enemy side.
const SIDE_ALLY: f32 = 1.0;
const SIDE_ENEMY: f32 = -1.0;
const EDITOR_MODE: usize = 0x1_403e_f36a;
const TEST_FIGHT_FLAG: usize = 0x1_47fc_6fb6;
const FIGHT_COMPARE_SITE: usize = 0x1_4013_d7b7;
const FIGHT_GENERATE_SITE: usize = 0x1_4013_dcf1;
const FIGHT_GENERATE_CONTINUE: usize = 0x1_4013_df38;
const FIGHT_LOAD_SITE: usize = 0x1_4013_e05e;
const MATCH_GUI_IDENTIFIER: usize = 0x1_4000_4a70;
const FIND_GUI_ELEMENT: usize = 0x1_401a_9170;
const TEST_DIFFICULTY_NAME: u64 = 0x1_4035_c9c8;
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
/// After the level loader's prologue, before it copies the requested path. This avoids the
/// entry hook used by Map Mode Extended; RCX still holds the path and RSP is 16-byte aligned.
const LEVEL_SEAM: usize = 0x1_4025_aff4;
/// Bound on the level path read at the loader seam; a longer name is not one of ours.
const LEVEL_PATH_LIMIT: usize = 64;
const LEVEL_BYTES: [u8; 10] = [
    0x41, 0x0f, 0xb6, 0xf8, // MOVZX EDI, R8B
    0x0f, 0xb6, 0xf2, // MOVZX ESI, DL
    0x48, 0x8b, 0xd9, // MOV RBX, RCX
];
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
const _: () = assert!(LINK_NODE_STRIDE == size_of::<Link>());
const LINK_TARGET: usize = offset_of!(Link, target);
/// End-pointer slot of a native `{begin, end, capacity}` vector header.
const VECTOR_END: usize = 8;
const LINK_COPY: usize = 0x1_4019_4b80;
const LINK_COPY_BYTES: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20];
/// Verified by design::verify; unregisters a Link at its actual storage address.
const LINK_UNLINK: usize = 0x1_4019_4d20;
/// Combat loader, immediately before it copies the enemy list into pre-combat.
/// The earlier enemy-loop guard does not protect that copy's subsequent consumers.
const PRECOMBAT_ENEMIES_SEAM: usize = 0x1_402c_3a98;
const PRECOMBAT_ENEMIES_BYTES: [u8; 7] = [0x48, 0x8d, 0x15, 0x21, 0x1f, 0x75, 0x03];
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

#[derive(Clone)]
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
    clickable: bool,
    /// Overrides the template text colour (the enemy list's red-dish hue).
    color: Option<u32>,
}

struct Session {
    entries: Vec<Entry>,
    enemy_entries: Vec<Entry>,
    status: Option<CString>,
    enemy_status: Option<CString>,
    serial: u64,
    panel_expanded: bool,
    last_panel_toggle_ms: Option<u64>,
    last_add_ms: Option<u64>,
    last_enemy_add_ms: Option<u64>,
    last_clear_ms: Option<u64>,
    last_ace_ms: Option<u64>,
    saved_mask: Option<u32>,
    airstrike: airstrike::Panel,
    defense: missile_defense::Panel,
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
    panel_expanded: false,
    last_panel_toggle_ms: None,
    last_add_ms: None,
    last_enemy_add_ms: None,
    last_clear_ms: None,
    last_ace_ms: None,
    saved_mask: None,
    airstrike: airstrike::Panel::new(),
    defense: missile_defense::Panel::new(),
    enemy_escadra: 0,
});
static READY: AtomicBool = AtomicBool::new(false);
static CONTROLS_VISIBLE: AtomicBool = AtomicBool::new(false);
static PANEL_TOGGLE_GUI: AtomicUsize = AtomicUsize::new(0);
static ADD_GUI: AtomicUsize = AtomicUsize::new(0);
static ENEMY_ADD_GUI: AtomicUsize = AtomicUsize::new(0);
static CLEAR_GUI: AtomicUsize = AtomicUsize::new(0);
static ACE_GUI: AtomicUsize = AtomicUsize::new(0);
static ACE_OPEN: AtomicBool = AtomicBool::new(false);
static ACE_OPTION_GUIS: [AtomicUsize; ACE_OPTIONS.len()] =
    [const { AtomicUsize::new(0) }; ACE_OPTIONS.len()];
static CRASH_GUI: AtomicUsize = AtomicUsize::new(0);
static CRASH_BUTTON: AtomicBool = AtomicBool::new(false);
static ROW_GUIS: [AtomicUsize; ROSTER_LIMIT] = [const { AtomicUsize::new(0) }; ROSTER_LIMIT];
static ENEMY_ROW_GUIS: [AtomicUsize; ENEMY_LIMIT] = [const { AtomicUsize::new(0) }; ENEMY_LIMIT];
static STATUS_GUI: AtomicUsize = AtomicUsize::new(0);
static ENEMY_STATUS_GUI: AtomicUsize = AtomicUsize::new(0);

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
        (LEVEL_SEAM, &LEVEL_BYTES[..]),
        (COPY_STRING, &COPY_STRING_BYTES[..]),
        (LINK_CLEAR_VECTOR, &LINK_CLEAR_VECTOR_BYTES[..]),
        (ENEMY_LOOP_SEAM, &ENEMY_LOOP_BYTES[..]),
        (PRECOMBAT_ENEMIES_SEAM, &PRECOMBAT_ENEMIES_BYTES[..]),
        (LINK_COPY, &LINK_COPY_BYTES[..]),
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
    // LEA RCX,[RIP+disp32] cannot be copied verbatim into a relocated trampoline:
    // it must still address ENEMY_LIST for the following native Link_ClearVector call.
    enemy_append.bytes(&[0x48, 0xb9]); // MOV RCX, imm64
    enemy_append.bytes(&ENEMY_LIST.to_le_bytes());
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

    let mut precombat_enemies = Trampoline::new();
    precombat_enemies.preserved_call(
        on_precombat_enemies as *const (),
        &[],
        &[],
        ReturnType::None,
    );
    // Replay the RIP-relative LEA as MOV RDX, imm64 at the relocated address.
    precombat_enemies.bytes(&[0x48, 0xba]);
    precombat_enemies.bytes(&ENEMY_LIST.to_le_bytes());
    precombat_enemies.absolute_jump(PRECOMBAT_ENEMIES_SEAM + PRECOMBAT_ENEMIES_BYTES.len());
    session
        .detour_trampoline(
            PRECOMBAT_ENEMIES_SEAM,
            PRECOMBAT_ENEMIES_BYTES.len(),
            precombat_enemies,
        )
        .map_err(|e| e.to_string())?;

    let mut level = Trampoline::new();
    level.preserved_call(on_level as *const (), &[], &[], ReturnType::None);
    level.bytes(&LEVEL_BYTES);
    level.absolute_jump(LEVEL_SEAM + LEVEL_BYTES.len());
    session
        .detour_trampoline(LEVEL_SEAM, LEVEL_BYTES.len(), level)
        .map_err(|e| e.to_string())?;
    install_fight_routing(session)?;
    missile_defense::install(session)?;
    airstrike::install(session)?;
    silent_strike::install(session)?;
    CRASH_BUTTON.store(crash_button, Ordering::Relaxed);
    READY.store(true, Ordering::Relaxed);
    Ok(())
}

/// Route accepted A.C.E. launch clicks through the native FIGHT builder.
///
/// # Safety
/// Install only during serialized initialization of the verified Steam 1.163 image.
unsafe fn install_fight_routing(session: &mut PatchSession) -> Result<(), String> {
    for (site, handler, result) in [
        (
            FIGHT_COMPARE_SITE,
            match_fight as *const (),
            ReturnType::Rax,
        ),
        (FIGHT_LOAD_SITE, load_fight as *const (), ReturnType::None),
    ] {
        let mut call = Trampoline::new();
        call.preserved_call(handler, &[], &[], result);
        call.absolute_jump(site + 5);
        session
            .detour_trampoline(site, 5, call)
            .map_err(|err| err.to_string())?;
    }

    // FIGHT's generator assumes its TEST slider and checkboxes exist. On the
    // skip path the native empty-list fallback and fleet builder still run.
    let mut generated = Trampoline::new();
    let skip = generated.new_label();
    generated.preserved_call(
        skip_enemy_generation as *const (),
        &[],
        &[],
        ReturnType::Rax,
    );
    generated.bytes(&[0x84, 0xc0]); // TEST AL,AL
    generated.jump_if(Condition::NotEqual, skip);
    generated.bytes(&[0x48, 0xb9]); // Relocate LEA RCX,[EDC_TEST_DIF] as MOV RCX,imm64.
    generated.bytes(&TEST_DIFFICULTY_NAME.to_le_bytes());
    generated.absolute_jump(FIGHT_GENERATE_SITE + 7);
    generated.bind(skip).map_err(|err| err.to_string())?;
    generated.absolute_jump(FIGHT_GENERATE_CONTINUE);
    session
        .detour_trampoline(FIGHT_GENERATE_SITE, 7, generated)
        .map_err(|err| err.to_string())?;
    Ok(())
}

unsafe extern "C" fn match_fight(name: usize, expected: *const i8) -> u8 {
    // SAFETY: ClickFunc supplies the live native identifier and comparison literal
    // on the game thread. Only a matching, pre-dispatch-approved click can alias FIGHT.
    let missile = missile_defense::requested();
    let airstrike = airstrike::requested();
    let silent = silent_strike::requested();
    if (missile || airstrike || silent)
        && native_string(name).is_some_and(|name| {
            (missile && name.as_c_str() == missile_defense::LAUNCH_NAME)
                || (airstrike && name.as_c_str() == airstrike::LAUNCH)
                || (silent && name.as_c_str() == silent_strike::BUTTON)
        })
    {
        return 1;
    }
    let matches: unsafe extern "C" fn(usize, *const i8) -> u8 =
        std::mem::transmute(MATCH_GUI_IDENTIFIER);
    matches(name, expected)
}

unsafe extern "C" fn skip_enemy_generation() -> bool {
    if missile_defense::requested() || airstrike::requested() {
        return true;
    }
    if !silent_strike::requested() {
        return false;
    }
    // SAFETY: FindGuiElement reads the live registry synchronously on the game
    // thread just as FIGHT would; these are static literals. We retain no pointers.
    let find: unsafe extern "C" fn(*const i8) -> usize = std::mem::transmute(FIND_GUI_ELEMENT);
    [
        c"EDC_TEST_DIF",
        c"EDC_TEST_SMALL",
        c"EDC_TEST_MID",
        c"EDC_TEST_LARGE",
        c"EDC_TEST_CARRIER",
    ]
    .into_iter()
    .any(|id| find(id.as_ptr()) == 0)
}

unsafe extern "C" fn load_fight(path: *const i8, resume: u8, flag: u8) {
    // SAFETY: this is ClickFunc's final FIGHT call, entered only after the native
    // builder has validated the design and linked both fleets. Its original level
    // path is the static string "Combat.seria"; the selected encounter may substitute it.
    if airstrike::requested() {
        airstrike::load_fight(resume, flag);
        return;
    }
    silent_strike::begin_combat();
    missile_defense::load_fight(path, resume, flag);
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
    let in_editor = read::<u8>(EDITOR_MODE) != 0;
    if !in_editor {
        ACE_OPEN.store(false, Ordering::Relaxed);
        if !CONTROLS_VISIBLE.load(Ordering::Relaxed) {
            return;
        }
    } else {
        missile_defense::cancel_unhandled_request();
        airstrike::cancel_unhandled_request();
        silent_strike::cancel_unhandled_request();
    }
    let scan = precombat::scan_gui_list();
    if !scan.complete {
        return;
    }
    let template = scan.elements.iter().copied().find(|&gui| {
        in_editor
            && read::<i32>(gui + OFF_ACTIVE) == 2
            && precombat::cstr_is(gui + offset_of!(Gui, identifier), b"EDC_TEST")
    });
    let Some(template) = template else {
        ACE_OPEN.store(false, Ordering::Relaxed);
        hide_control(
            PANEL_TOGGLE_NAME.to_bytes_with_nul(),
            &PANEL_TOGGLE_GUI,
            &scan.elements,
        );
        hide_panel(&scan.elements);
        CONTROLS_VISIBLE.store(false, Ordering::Relaxed);
        return;
    };
    let Ok(mut state) = SESSION.try_lock() else {
        return;
    };
    let width = read::<f32>(template + 0x218);
    let height = read::<f32>(template + 0x21c);
    let test_right = read::<f32>(template + offset_of!(Body, position_x)) + width * 0.5;
    let y = read::<f32>(template + offset_of!(Body, position_y));
    if ![test_right, y, width, height].iter().all(|v| v.is_finite())
        || width <= 0.0
        || height <= 0.0
    {
        return;
    }
    // Buttons use center coordinates. Keep TEST in place and insert a narrow toggle after it.
    let toggle_width = height.min(width * 0.5);
    let toggle_left = test_right + BUTTON_GAP;
    show_control(
        PANEL_TOGGLE_NAME.to_bytes_with_nul(),
        &PANEL_TOGGLE_GUI,
        &scan.elements,
        template,
        if state.panel_expanded { c"<" } else { c">" },
        ControlGeometry {
            position: [toggle_left + toggle_width * 0.5, y],
            dimensions: [toggle_width, height],
        },
        BUTTON_STYLE,
    );
    if !state.panel_expanded {
        hide_panel(&scan.elements);
        CONTROLS_VISIBLE.store(true, Ordering::Relaxed);
        return;
    }
    let ace_open = ACE_OPEN.load(Ordering::Relaxed);
    if !ace_open {
        hide_ace_options(&scan.elements);
    }
    let x = toggle_left + toggle_width + BUTTON_GAP + width * 0.5;
    let button_size = [width, height];
    let button = |position: [f32; 2]| ControlGeometry {
        position,
        dimensions: button_size,
    };
    show_control(
        ADD_NAME,
        &ADD_GUI,
        &scan.elements,
        template,
        c"ADD",
        button([x, y]),
        BUTTON_STYLE,
    );
    show_control(
        ENEMY_ADD_NAME,
        &ENEMY_ADD_GUI,
        &scan.elements,
        template,
        c"ADD ENEMY",
        button([x + width + BUTTON_GAP, y]),
        BUTTON_STYLE,
    );
    show_control(
        CLEAR_NAME,
        &CLEAR_GUI,
        &scan.elements,
        template,
        c"CLEAR",
        button([x + 2.0 * (width + BUTTON_GAP), y]),
        BUTTON_STYLE,
    );
    let ace_x = x + 3.0 * (width + BUTTON_GAP);
    show_control(
        ACE_NAME.to_bytes_with_nul(),
        &ACE_GUI,
        &scan.elements,
        template,
        c"A.C.E.",
        button([ace_x, y]),
        BUTTON_STYLE,
    );
    let mut toolbar_right = ace_x + width * 0.5;
    if CRASH_BUTTON.load(Ordering::Relaxed) {
        let crash_x = ace_x + width + BUTTON_GAP;
        toolbar_right = crash_x + width * 0.5;
        show_control(
            CRASH_NAME,
            &CRASH_GUI,
            &scan.elements,
            template,
            c"CRASH",
            button([crash_x, y]),
            BUTTON_STYLE,
        );
    }
    if ace_open {
        // Give the longer labels two button widths, lined up with A.C.E.'s left edge.
        for (index, ((name, text), kept)) in ACE_OPTIONS.iter().zip(&ACE_OPTION_GUIS).enumerate() {
            show_control(
                name.to_bytes_with_nul(),
                kept,
                &scan.elements,
                template,
                text,
                ControlGeometry {
                    position: [
                        ace_x + width * 0.5,
                        y + (index + 1) as f32 * (height + ACE_OPTION_GAP),
                    ],
                    dimensions: [2.0 * width, height],
                },
                BUTTON_STYLE,
            );
        }
    }
    state
        .defense
        .draw(&scan.elements, template, [width, height], y, toolbar_right);
    state.airstrike.draw(
        &scan.elements,
        template,
        [width, height],
        y,
        toolbar_right,
        ace_open,
    );

    // One list column below the buttons: ally rows, ally status, enemy rows, enemy status.
    let left = x - width * 0.5;
    let list_top = y + height * 0.5 + LIST_TOP_GAP;
    let row = |index: usize| ControlGeometry {
        position: [left, list_top + index as f32 * ROW_HEIGHT],
        dimensions: [LIST_WIDTH, ROW_HEIGHT],
    };
    for (index, entry) in state.entries.iter().enumerate() {
        show_control(
            ROW_NAMES[index],
            &ROW_GUIS[index],
            &scan.elements,
            template,
            &entry.name,
            row(index),
            ROW_STYLE,
        );
    }
    for (&name, gui) in ROW_NAMES.iter().zip(&ROW_GUIS).skip(state.entries.len()) {
        hide_control(name, gui, &scan.elements);
    }
    let mut next_row = state.entries.len();
    if let Some(status) = &state.status {
        show_control(
            STATUS_NAME,
            &STATUS_GUI,
            &scan.elements,
            template,
            status,
            row(next_row),
            ROW_STYLE,
        );
        next_row += 1;
    } else {
        hide_control(STATUS_NAME, &STATUS_GUI, &scan.elements);
    }
    for (index, entry) in state.enemy_entries.iter().enumerate() {
        show_control(
            ENEMY_ROW_NAMES[index],
            &ENEMY_ROW_GUIS[index],
            &scan.elements,
            template,
            &entry.name,
            row(next_row + index),
            ENEMY_ROW_STYLE,
        );
    }
    for (&name, gui) in ENEMY_ROW_NAMES
        .iter()
        .zip(&ENEMY_ROW_GUIS)
        .skip(state.enemy_entries.len())
    {
        hide_control(name, gui, &scan.elements);
    }
    next_row += state.enemy_entries.len();
    if let Some(status) = &state.enemy_status {
        show_control(
            ENEMY_STATUS_NAME,
            &ENEMY_STATUS_GUI,
            &scan.elements,
            template,
            status,
            row(next_row),
            ENEMY_ROW_STYLE,
        );
    } else {
        hide_control(ENEMY_STATUS_NAME, &ENEMY_STATUS_GUI, &scan.elements);
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
    let background_names = [
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
    let (Some(back), Some(focus), Some(pressed), Some(font)) = (
        &background_names[0],
        &background_names[1],
        &background_names[2],
        font,
    ) else {
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
    write(gui + offset_of!(Body, focusable), u8::from(style.clickable));
    write(gui + offset_of!(Gui, clickable), u8::from(style.clickable));
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
        if style.clickable {
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

/// # Safety
/// `listed` is the current complete game-thread scan, and `name` includes its NUL.
unsafe fn hide_control(name: &[u8], kept: &AtomicUsize, listed: &[usize]) {
    let gui = kept.load(Ordering::Relaxed);
    // SAFETY: Only a listed, active GUI with this exact identifier is written. The game
    // retains the object and its resources; this only changes its active state.
    if precombat::listed_shown(listed, gui, name) {
        precombat::set_gui_active(gui, false);
    }
}

/// Hide only active options verified against this input callback's complete GUI scan.
unsafe fn hide_ace_options(listed: &[usize]) {
    for ((name, _), kept) in ACE_OPTIONS.iter().zip(&ACE_OPTION_GUIS) {
        hide_control(name.to_bytes_with_nul(), kept, listed);
    }
}

unsafe fn hide_panel(listed: &[usize]) {
    hide_control(ACE_NAME.to_bytes_with_nul(), &ACE_GUI, listed);
    hide_ace_options(listed);
    airstrike::hide(listed);
    missile_defense::hide(listed);
    hide_control(ADD_NAME, &ADD_GUI, listed);
    hide_control(ENEMY_ADD_NAME, &ENEMY_ADD_GUI, listed);
    hide_control(CLEAR_NAME, &CLEAR_GUI, listed);
    hide_control(CRASH_NAME, &CRASH_GUI, listed);
    hide_control(STATUS_NAME, &STATUS_GUI, listed);
    hide_control(ENEMY_STATUS_NAME, &ENEMY_STATUS_GUI, listed);
    for (&name, gui) in ROW_NAMES.iter().zip(&ROW_GUIS) {
        hide_control(name, gui, listed);
    }
    for (&name, gui) in ENEMY_ROW_NAMES.iter().zip(&ENEMY_ROW_GUIS) {
        hide_control(name, gui, listed);
    }
}

/// Called from the existing native click dispatcher, on the game thread.
pub(crate) unsafe fn on_click(name: &str, button: usize) -> bool {
    if !READY.load(Ordering::Relaxed) {
        return false;
    }
    let panel_toggle = name.as_bytes() == PANEL_TOGGLE_NAME.to_bytes();
    let ace_button = name.as_bytes() == ACE_NAME.to_bytes();
    let ace_option = ACE_OPTIONS
        .iter()
        .any(|(id, _)| name.as_bytes() == id.to_bytes());
    let left_click = button as u32 == 1;
    let closed_ace = left_click && !ace_button && ACE_OPEN.swap(false, Ordering::Relaxed);
    if name == "EDC_TEST_FIGHT" {
        missile_defense::cancel_request();
        airstrike::cancel_request();
        silent_strike::cancel_request();
        return false;
    }
    let defense_control = missile_defense::is_control(name);
    let air_control = airstrike::is_control(name);
    if !panel_toggle
        && !ace_button
        && !ace_option
        && !defense_control
        && !air_control
        && !matches!(
            name,
            "SW_ALLY_ADD" | "SW_ENEMY_ADD" | "SW_ALLY_CLEAR" | "SW_ALLY_CRASH"
        )
    {
        return false;
    }
    if !left_click {
        return true;
    }
    if read::<u8>(EDITOR_MODE) == 0 || !CONTROLS_VISIBLE.load(Ordering::Relaxed) {
        return true;
    }
    let Ok(mut state) = SESSION.try_lock() else {
        return true;
    };
    if panel_toggle {
        let now = now_ms();
        if state
            .last_panel_toggle_ms
            .is_some_and(|last| now.wrapping_sub(last) <= CLICK_DEBOUNCE_MS)
        {
            return true;
        }
        state.last_panel_toggle_ms = Some(now);
        state.panel_expanded = !state.panel_expanded;
        drop(state);
        sound::button("Shipworks tools toggle");
        return true;
    }
    // Consume stale custom click events before they can act on hidden controls.
    if !state.panel_expanded {
        return true;
    }
    if ace_option {
        if !closed_ace {
            return true;
        }
        if name.as_bytes() == ACE_DEFENSE_NAME.to_bytes() {
            state.airstrike.close();
            state.defense.open();
        } else {
            state.defense.close();
            if name.as_bytes() == airstrike::BUTTON.to_bytes() {
                state.airstrike.open();
            } else {
                state.airstrike.close();
            }
        }
        drop(state);
        sound::button("Shipworks A.C.E. option");
        if name.as_bytes() == silent_strike::BUTTON.to_bytes() {
            missile_defense::cancel_request();
            airstrike::cancel_request();
            silent_strike::request();
            // Let the native dispatcher reach the FIGHT comparison with this identifier.
            return false;
        }
        return true;
    }
    if name == "SW_ALLY_CRASH" {
        drop(state);
        if CRASH_BUTTON.load(Ordering::Relaxed) {
            log::error!("hf_ally: deliberate Shipworks fail-fast requested");
            // SAFETY: Both strings are static and NUL-terminated; a null owner is valid.
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxA(
                std::ptr::null_mut(),
                c"What did you expect?\nDisable this button in ally.json"
                    .as_ptr()
                    .cast(),
                c"CRASH".as_ptr().cast(),
                windows_sys::Win32::UI::WindowsAndMessaging::MB_OK
                    | windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONWARNING
                    | windows_sys::Win32::UI::WindowsAndMessaging::MB_TOPMOST,
            );
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
    if defense_control {
        let has_allies = !state.entries.is_empty();
        let has_enemies = !state.enemy_entries.is_empty();
        // A valid LAUNCH falls through to the native click dispatcher and aliases FIGHT.
        return !state.defense.click(name, has_allies, has_enemies);
    }
    if air_control {
        let allies = !state.entries.is_empty();
        let enemies = !state.enemy_entries.is_empty();
        return !state.airstrike.click(name, allies, enemies);
    }
    let now = now_ms();
    let clear = name == "SW_ALLY_CLEAR";
    let enemy_add = name == "SW_ENEMY_ADD";
    let last_click_ms = if ace_button {
        &mut state.last_ace_ms
    } else if clear {
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
    if ace_button {
        drop(state);
        ACE_OPEN.fetch_xor(true, Ordering::Relaxed);
        sound::button("Shipworks A.C.E.");
        return true;
    }
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
            damage::invalidate_replay();
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
    if missile_defense::requested() || airstrike::requested() {
        return;
    }
    let Ok(mut state) = SESSION.try_lock() else {
        return;
    };
    let mut added = 0;
    for entry in &state.entries {
        if design::append(fleet, &entry.path, SIDE_ALLY).is_some() {
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
    log::info!("hf_ally: Shipworks FIGHT added {added} allies");
}

type CreateEscadra = unsafe extern "C" fn(usize, f32, u32, usize, usize, u32) -> usize;

/// Remove dead fleet links before native pre-combat copies them into its own vectors.
/// This is needed even when neither custom Shipworks roster has ever been used.
unsafe extern "C" fn on_precombat_enemies() {
    let _hook = diag::scope("Shipworks precombat enemy cleanup");
    if read::<u8>(TEST_FIGHT_FLAG) == 0 {
        return;
    }
    retain_enemy_fleets(0);
}

/// Keep live enemy fleets except `excluded`, preserving order and native Link registrations.
///
/// # Safety
/// Game-thread fleet lifecycle callbacks only; ENEMY_LIST is the native-owned Link vector.
/// Node destruction invalidates its Links. A non-null target here remains game-owned and
/// live for this synchronous operation; page checks only validate container bounds.
unsafe fn retain_enemy_fleets(excluded: usize) {
    let Some(begin) = diag::rd::<usize>(ENEMY_LIST) else {
        return;
    };
    let Some(end) = diag::rd::<usize>(ENEMY_LIST + VECTOR_END) else {
        return;
    };
    if begin == 0 || end < begin {
        return;
    }
    let bytes = end - begin;
    if !bytes.is_multiple_of(LINK_NODE_STRIDE) || bytes / LINK_NODE_STRIDE > LIST_ENTRY_LIMIT {
        return;
    }
    if !diag::readable_span(begin, bytes) {
        return;
    }
    let unlink: unsafe extern "C" fn(usize) = std::mem::transmute(LINK_UNLINK);
    let copy: unsafe extern "C" fn(usize, usize) -> usize = std::mem::transmute(LINK_COPY);
    let mut kept = begin;
    let mut source = begin;
    while source < end {
        let fleet = read::<usize>(source + LINK_TARGET);
        if fleet != 0 && fleet != excluded {
            if kept != source {
                // SAFETY: distinct, initialized native Link slots in the validated vector.
                // Unregister the old destination before constructing the copy there. The
                // source remains registered until overwritten or destroyed in the tail.
                unlink(kept);
                copy(kept, source);
            }
            kept += LINK_NODE_STRIDE;
        }
        source += LINK_NODE_STRIDE;
    }
    if kept == end {
        return;
    }
    let mut tail = kept;
    while tail < end {
        // SAFETY: every tail slot is still initialized; release its native registration
        // before shortening the vector. No owning Rust Link copies or byte moves.
        unlink(tail);
        tail += LINK_NODE_STRIDE;
    }
    write::<usize>(ENEMY_LIST + VECTOR_END, kept);
    log::info!(
        "hf_ally: removed {} dead/excluded enemy fleet links; {} live fleets remain",
        (end - kept) / LINK_NODE_STRIDE,
        (kept - begin) / LINK_NODE_STRIDE,
    );
}

/// One-line description of the escadras of a native pointer vector: `count` cells of `stride` bytes,
/// the escadra pointer at `value_offset` in each cell, `stored` marked with `*`. Only pointer values
/// are compared; every read is page-checked and count-bounded.
///
/// # Safety
/// Call on the game thread with a live native vector's begin/end values, a nonzero `stride`,
/// and a pointer field within each cell. Reads go through `diag::rd`.
unsafe fn describe_escadras(
    begin: usize,
    end: usize,
    stride: usize,
    value_offset: usize,
    stored: usize,
) -> (usize, String) {
    let mut description = String::new();
    if begin == 0 || end < begin {
        return (0, description);
    }
    let count = (end - begin) / stride;
    if count > LIST_ENTRY_LIMIT {
        return (0, description);
    }
    for index in 0..count {
        let fleet = diag::rd::<usize>(begin + index * stride + value_offset).unwrap_or(0);
        let ships = diag::rd::<usize>(fleet.wrapping_add(ESCADRA_SHIP_COUNT)).unwrap_or(0);
        let _ = write!(
            description,
            " {fleet:#x}({}{ships})",
            if fleet == stored { "*" } else { "" },
        );
    }
    (count, description)
}

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

    let profile_begin = diag::rd::<usize>(profile.wrapping_add(PROFILE_ESCADRAS)).unwrap_or(0);
    let profile_end =
        diag::rd::<usize>(profile.wrapping_add(PROFILE_ESCADRAS + VECTOR_END)).unwrap_or(0);
    let (profile_count, profile_entries) =
        describe_escadras(profile_begin, profile_end, size_of::<usize>(), 0, stored);

    let list_begin = diag::rd::<usize>(ENEMY_LIST).unwrap_or(0);
    let list_end = diag::rd::<usize>(ENEMY_LIST + VECTOR_END).unwrap_or(0);
    let (list_count, list_entries) =
        describe_escadras(list_begin, list_end, LINK_NODE_STRIDE, LINK_TARGET, stored);

    log::info!(
        "hf_ally: enemy state {tag}: stored={stored:#x} profile[{profile_count}]{profile_entries} list[{list_count}]{list_entries}"
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
    prune_profile_escadra(profile, escadra);
    retain_enemy_fleets(escadra);
}

/// Drop `escadra` from the profile's escadra vector and close the gap by moving the remaining
/// 8-byte cells. The vector header and the cell range are validated first.
///
/// # Safety
/// Game thread only. Only the vector's own bounds and pointer values are read or written.
unsafe fn prune_profile_escadra(profile: usize, escadra: usize) {
    let begin = diag::rd::<usize>(profile.wrapping_add(PROFILE_ESCADRAS)).unwrap_or(0);
    let end = diag::rd::<usize>(profile.wrapping_add(PROFILE_ESCADRAS + VECTOR_END)).unwrap_or(0);
    let bytes = end.wrapping_sub(begin);
    if begin == 0
        || end < begin
        || !bytes.is_multiple_of(8)
        || bytes > ESCADRA_VECTOR_LIMIT
        || !diag::readable_span(begin, bytes)
    {
        return;
    }
    let mut cell = begin;
    while cell < end {
        if read::<usize>(cell) != escadra {
            cell += 8;
            continue;
        }
        let mut source = cell + 8;
        while source < end {
            write::<usize>(source - 8, read::<usize>(source));
            source += 8;
        }
        write::<usize>(profile + PROFILE_ESCADRAS + VECTOR_END, end - 8);
        log::info!("hf_ally: pruned previous test enemy escadra {escadra:#x} from the profile escadra vector");
        return;
    }
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
    let custom = missile_defense::requested()
        || airstrike::requested()
        || SESSION
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
    let bogus_name = c"HF_NO_SUCH_SHIP".to_bytes_with_nul();
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
    if missile_defense::requested() || airstrike::requested() {
        return;
    }
    let Ok(state) = SESSION.try_lock() else {
        return;
    };
    let mut added = 0;
    for entry in &state.enemy_entries {
        if design::append(escadra, &entry.path, SIDE_ENEMY).is_some() {
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
    ACE_OPEN.store(false, Ordering::Relaxed);
    let mut level_path = Vec::new();
    for index in 0..LEVEL_PATH_LIMIT {
        let Some(byte) = diag::rd::<u8>(path + index) else {
            return;
        };
        if byte == 0 {
            break;
        }
        level_path.push(byte);
    }
    // A test/landing trip and its return are still the same Shipworks session.
    if level_path != b"Combat.seria" {
        silent_strike::leave();
    }
    if level_path == b"Missile.seria" && missile_defense::active() {
        return;
    }
    if level_path == airstrike::LEVEL.to_bytes() && airstrike::active() {
        return;
    }
    let airstrike_return = airstrike::active();
    airstrike::leave();
    let missile_return = missile_defense::active();
    missile_defense::leave();
    if level_path == b"Combat.seria"
        || (level_path == b"City_generated.seria" && read::<u8>(TEST_FIGHT_FLAG) != 0)
    {
        return;
    }
    let Ok(mut state) = SESSION.try_lock() else {
        return;
    };
    if let Some(mask) = state.saved_mask.take() {
        precombat::replace_selected_mask(mask);
    }
    if level_path == b"Editor.seria" {
        if !missile_return {
            state.defense.close();
        }
        if !airstrike_return {
            state.airstrike.close();
        }
        return;
    }
    state.airstrike = airstrike::Panel::new();
    state.defense = missile_defense::Panel::new();
    state.panel_expanded = false;
    state.last_panel_toggle_ms = None;
    clear_roster(&mut state);
    state.last_clear_ms = None;
    state.last_ace_ms = None;
    drop(state);
    dump_enemy_state("leaving shipworks");
    prune_enemy_escadra();
}

/// Drop both rosters' snapshots and statuses; keep the editor ship and capture serial intact.
/// Called for both explicit CLEAR and leaving Shipworks.
fn clear_roster(state: &mut Session) {
    damage::clear();
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
