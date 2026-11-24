//! Shipworks-only adapter for the native SCR_MISSILE scene. See
//! docs/shipworks-test-allies.md for the Steam 1.163 call seams and ownership.

use std::{
    ffi::{CStr, CString},
    mem::{offset_of, size_of},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
};

use highfleet::general::SentinelListNode;
use patchy::{Condition, PatchSession, ReturnType, Trampoline};

use crate::{
    diag,
    native::{read, SetLinkFn, OFF_AI_FLAG, OFF_SIDE, ROSTER_BEGIN, SET_LINK},
    native_layout::{Body, Creature, EscadraString, Node, PartStats, SentinelList, NODE_CODE},
    precombat, sound,
    time::now_ms,
};

use super::{
    native_string, prune_enemy_escadra, show_control, write, ControlGeometry, BUTTON_STYLE,
    CLICK_DEBOUNCE_MS, ENEMY_LIMIT, ENEMY_LIST, LINK_CLEAR_VECTOR, LINK_NODE_STRIDE, LINK_TARGET,
    ROSTER_LIMIT, SIDE_ALLY, SIDE_ENEMY, TEST_FIGHT_FLAG,
};

const TYPE_NAME: &CStr = c"SW_ACE_MSL";
const START_NAME: &CStr = c"SW_ACE_START";
const LAUNCH_NAME: &CStr = c"SW_ACE_LAUNCH";
const MISSILE_LEVEL: &CStr = c"Missile.seria";
const EDITOR_LEVEL: &CStr = c"Editor.seria";
const FRAME_WIDTH: usize = 0x1_43a1_58fc;
const PANEL_RIGHT_MARGIN: f32 = 36.0;
const PANEL_TOP_GAP: f32 = 32.0;
const PANEL_ROW_GAP: f32 = 8.0;
static TYPE_GUI: AtomicUsize = AtomicUsize::new(0);
static START_GUI: AtomicUsize = AtomicUsize::new(0);
static LAUNCH_GUI: AtomicUsize = AtomicUsize::new(0);

const STATS_TABLE: usize = 0x1_43a3_3bf0;
const STATS_LIMIT: usize = 4096;
// Stats_GetIndexMap (0x140281fa0) uses the first opaque EscadraString as the OID.
const STATS_OID: usize = offset_of!(PartStats, unknown_0048h);
const STATS_HOMING_TYPE: usize = 0x164;
const STATS_EXPLOSIVE: usize = 0x168;
const STRATEGIC_CATEGORY: u32 = 0x0800_0000;
const NUCLEAR_YIELD: f32 = 100_000.0;
// The ordinary tactical blast path was not designed for arbitrary strategic
// yields. Use the stock OL.seria nuclear yield as the sandbox ceiling.
const MAX_TACTICAL_NUCLEAR_YIELD: f32 = 40_000_000.0;
const ALL_PARTS: usize = 0x1_43a1_39f0;
const NODE_FIND_OID: usize = 0xc8;
const LOOKUP_LABEL: usize = 0x1_401a_e090;
const STRING_DTOR: usize = 0x1_4000_6d70;

const FRIENDLY_LIST: usize = 0x1_43a1_58e0;
const ESCADRA_CODE: u64 = 0x147;
const CREATURE_CODE: u64 = 0x2f;
const MISSILE_CODE: u64 = 0x4000_0000_1047;
const NEW: usize = 0x1_4033_4094;
const MISSILE_CTOR: usize = 0x1_400e_48a0;
const MISSILE_DTOR: usize = 0x1_400e_81d0;
const MISSILE_SIZE: usize = 0x308;
const MAP_SIDE: usize = 0x128;
const MAP_MISSILE_COUNT: usize = 0x248;
const MAP_MISSILE_HOMING_TYPE: usize = 0x29c;
const MAP_MISSILE_OID: usize = 0x2b8;
const MAP_MISSILE_EXPLOSIVE: usize = 0x2f0;
// Map seeker (0x14010f1f0, branch 0x14010f383): homingType == 2 ("P" / anti-radiation)
// homes only on active radar (Escadra+0x4d0, from Creature+0x17a0) and jammer
// (Escadra+0x4e0, from Creature+0x17c4) emissions.
const HOMING_ANTI_RADIATION: i32 = 2;
const CREATURE_RADAR: usize = 0x17a0;
const CREATURE_JAMMER: usize = 0x17c4;
const TACTICAL_MISSILE_SIDE: usize = 0x248;
const PUSH_LINK: usize = 0x1_4004_af20;
const LOAD_LEVEL: usize = 0x1_4025_afd0;
const MATCH_NAME: usize = 0x1_4000_4a70;
const SPAWN_DEFENDERS: usize = 0x1_402d_45c0;
const ASSIGN_DEFENDER: usize = 0x1_4026_3700;
const HGE: u64 = 0x1_43a1_19e0;
const PROPERTY_SHIP: usize = 0x1_4021_b070;
const HAS_THRUST: usize = 0x1_4007_3450;
const VANILLA_DEFENDER_LIMIT: u32 = 4;
const CUSTOM_DEFENDER_LIMIT: usize = 1 + ROSTER_LIMIT + ENEMY_LIMIT;
const ARENA_RIGHT: usize = 0x1_403c_2bc0;
const ARENA_BOTTOM: usize = 0x1_403c_2bc4;
const ARENA_LEFT: usize = 0x1_403c_2bc8;
const FROM_RIGHT: usize = 0x1_403c_2b69;
const GROUND_EDGE_MARGIN: f32 = 450.0;
const GROUND_MAX_SPACING: f32 = 750.0;
const EXTRA_DEFENDERS_PER_ROW: u32 = 5;
const EXTRA_DEFENDER_HEIGHT: f32 = -800.0;
const EXTRA_ROW_HEIGHT: f32 = 320.0;
const NUCLEAR_EFFECT: usize = 0x1_4002_bb50;
const NUCLEAR_MAP_IMPACT: usize = 0x1_400f_11b0;
const SHADER_MAP_TECHNIQUE: usize = 0x118;
const SHADER_UNWARPED_TECHNIQUE: usize = 0x120;

const _: () = {
    assert!(STATS_OID == 0x48);
    assert!(size_of::<EscadraString>() == 0x20);
    assert!(offset_of!(Body, logic_active) == 0x3c8);
    assert!(offset_of!(Body, explosive_power) == 0x2dc);
    assert!(offset_of!(SentinelListNode<usize>, value) == 0x10);
    assert!(CREATURE_RADAR + size_of::<f32>() <= size_of::<Creature>());
    assert!(CREATURE_JAMMER + size_of::<f32>() <= size_of::<Creature>());
};

struct Missile {
    oid: CString,
    label: CString,
    homing_type: i32,
    explosive: f32,
}

#[derive(Clone, Copy, Debug)]
enum Roster {
    Defense,
    AlliedDefense,
    Strike,
    Mixed,
}

impl Roster {
    fn missile_side(self) -> f32 {
        match self {
            Self::Strike => SIDE_ALLY,
            Self::Defense | Self::AlliedDefense | Self::Mixed => SIDE_ENEMY,
        }
    }
}

struct Launch {
    missile: Missile,
    roster: Roster,
    start: DefenderStart,
}

#[derive(Clone, Copy, Debug)]
enum DefenderStart {
    Airborne,
    Ground,
}

impl DefenderStart {
    fn label(self) -> &'static CStr {
        match self {
            Self::Airborne => c"AIRBORNE",
            Self::Ground => c"GROUND",
        }
    }
}

pub(super) struct Panel {
    open: bool,
    choices: Vec<Missile>,
    selected: usize,
    start: DefenderStart,
    last_click: Option<(&'static CStr, u64)>,
}

static REQUEST: Mutex<Option<Launch>> = Mutex::new(None);
static BATTLE: Mutex<Option<Launch>> = Mutex::new(None);
static ACTIVE: AtomicBool = AtomicBool::new(false);
// State 1's synchronous spawner: zero selects AIRBORNE, otherwise this is
// the number of GROUND ships to space along the terrain.
static GROUNDED_DEFENDERS: AtomicUsize = AtomicUsize::new(0);
// A marker, never followed to the object. Only a matching live entry in side B
// grants access to our temporary map missile.
static OWN_MISSILE: AtomicUsize = AtomicUsize::new(0);

impl Panel {
    pub(super) const fn new() -> Self {
        Self {
            open: false,
            choices: Vec::new(),
            selected: 0,
            start: DefenderStart::Airborne,
            last_click: None,
        }
    }

    /// Called from Shipworks input, with the game and its libraries initialized.
    pub(super) unsafe fn open(&mut self) {
        // SAFETY: this is the editor input thread; no native library refresh
        // runs concurrently. `catalog` copies the display strings/keys.
        let choices = catalog();
        self.selected = self
            .choices
            .get(self.selected)
            .and_then(|old| choices.iter().position(|new| new.oid == old.oid))
            .or_else(|| {
                choices
                    .iter()
                    .position(|missile| missile.explosive <= NUCLEAR_YIELD)
            })
            .unwrap_or(0);
        self.choices = choices;
        self.last_click = None;
        self.open = true;
        log::info!(
            "hf_ally: Shipworks Missile Defense found {} launchable strategic missile types",
            self.choices.len()
        );
    }

    pub(super) fn close(&mut self) {
        self.open = false;
    }

    /// Draw against the validated Shipworks TEST template and current GUI list.
    pub(super) unsafe fn draw(
        &self,
        listed: &[usize],
        template: usize,
        [width, height]: [f32; 2],
        top: f32,
    ) {
        if !self.open {
            hide(listed);
            return;
        }
        let frame_width = read::<i32>(FRAME_WIDTH) as f32;
        let panel_width = (width * 3.2).min(frame_width - 2.0 * PANEL_RIGHT_MARGIN);
        if panel_width <= 0.0 {
            hide(listed);
            return;
        }
        let x = (frame_width - panel_width) * 0.5 - PANEL_RIGHT_MARGIN;
        let y = top + height + PANEL_TOP_GAP;
        let row = |y| ControlGeometry {
            position: [x, y],
            dimensions: [panel_width, height],
        };
        let missile = self.choices.get(self.selected);
        show_control(
            TYPE_NAME.to_bytes_with_nul(),
            &TYPE_GUI,
            listed,
            template,
            missile.map_or(c"NO STRATEGIC MISSILES", |missile| &missile.label),
            row(y),
            BUTTON_STYLE,
        );
        if missile.is_some() {
            show_control(
                START_NAME.to_bytes_with_nul(),
                &START_GUI,
                listed,
                template,
                self.start.label(),
                row(y + height + PANEL_ROW_GAP),
                BUTTON_STYLE,
            );
            show_control(
                LAUNCH_NAME.to_bytes_with_nul(),
                &LAUNCH_GUI,
                listed,
                template,
                c"LAUNCH",
                row(y + 2.0 * (height + PANEL_ROW_GAP)),
                BUTTON_STYLE,
            );
        } else {
            hide_button(listed, START_NAME, &START_GUI);
            hide_button(listed, LAUNCH_NAME, &LAUNCH_GUI);
        }
    }

    /// True only when the native dispatcher should run its test-fight branch.
    pub(super) unsafe fn click(&mut self, name: &str, allies: bool, enemies: bool) -> bool {
        if !self.open || self.choices.is_empty() || active() {
            return false;
        }
        let Some(id) = control_id(name) else {
            return false;
        };
        let now = now_ms();
        if self.last_click.is_some_and(|(last_id, last_ms)| {
            last_id == id && now.wrapping_sub(last_ms) <= CLICK_DEBOUNCE_MS
        }) {
            return false;
        }
        self.last_click = Some((id, now));
        if id == TYPE_NAME {
            self.selected = (self.selected + 1) % self.choices.len();
            sound::button("Shipworks missile type");
            return false;
        }
        if id == START_NAME {
            self.start = match self.start {
                DefenderStart::Airborne => DefenderStart::Ground,
                DefenderStart::Ground => DefenderStart::Airborne,
            };
            sound::button("Shipworks missile defender start");
            return false;
        }
        let Some(selected) = self.choices.get(self.selected) else {
            return false;
        };
        // The key might have vanished from a modded live table since the panel opened.
        let Some(missile) = catalog()
            .into_iter()
            .find(|choice| choice.oid == selected.oid)
        else {
            self.open();
            return false;
        };
        let roster = match (allies, enemies) {
            (false, false) => Roster::Defense,
            (true, false) => Roster::AlliedDefense,
            (false, true) => Roster::Strike,
            (true, true) => Roster::Mixed,
        };
        let Ok(mut request) = REQUEST.try_lock() else {
            return false;
        };
        *request = Some(Launch {
            missile,
            roster,
            start: self.start,
        });
        drop(request);
        sound::button("Shipworks missile launch");
        true
    }
}

pub(super) fn is_control(name: &str) -> bool {
    control_id(name).is_some()
}

fn control_id(name: &str) -> Option<&'static CStr> {
    [TYPE_NAME, START_NAME, LAUNCH_NAME]
        .into_iter()
        .find(|id| id.to_bytes() == name.as_bytes())
}

unsafe fn hide_button(listed: &[usize], name: &CStr, slot: &AtomicUsize) {
    let gui = slot.load(Ordering::Relaxed);
    // SAFETY: only alter active members of this game-thread scan with the exact identifier.
    if precombat::listed_shown(listed, gui, name.to_bytes_with_nul()) {
        precombat::set_gui_active(gui, false);
    }
}

pub(super) unsafe fn hide(listed: &[usize]) {
    hide_button(listed, TYPE_NAME, &TYPE_GUI);
    hide_button(listed, START_NAME, &START_GUI);
    hide_button(listed, LAUNCH_NAME, &LAUNCH_GUI);
}

pub(super) fn requested() -> bool {
    REQUEST.try_lock().is_ok_and(|request| request.is_some())
}

pub(super) fn cancel_request() {
    if let Ok(mut request) = REQUEST.try_lock() {
        *request = None;
    }
}

/// FIGHT runs synchronously after the click. A request left at the next
/// input pass never made it through native ship validation.
pub(super) fn cancel_unhandled_request() {
    if let Ok(mut request) = REQUEST.try_lock() {
        if request.take().is_some() {
            log::warn!("hf_ally: Shipworks Missile Defense FIGHT did not accept the editor ship");
        }
    }
}

pub(super) fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Read a native vector header and bound the borrowed element span. Game thread only.
unsafe fn vector_span(header: usize, stride: usize, limit: usize) -> Option<(usize, usize)> {
    let begin = diag::rd::<usize>(header)?;
    let end = diag::rd::<usize>(header + size_of::<usize>())?;
    let capacity = diag::rd::<usize>(header + 2 * size_of::<usize>())?;
    let bytes = end.checked_sub(begin)?;
    if begin == 0
        || !begin.is_multiple_of(size_of::<usize>())
        || end > capacity
        || !bytes.is_multiple_of(stride)
        || bytes / stride > limit
        || !diag::readable_span(begin, bytes)
    {
        return None;
    }
    Some((begin, bytes / stride))
}

/// Copy strategic definitions from the live stats table, excluding records with
/// no actual spawnable Body in ALL_PARTS (the library DefinePart uses).
unsafe fn catalog() -> Vec<Missile> {
    let mut choices: Vec<Missile> = Vec::new();
    let Some((table, count)) = vector_span(STATS_TABLE, size_of::<usize>(), STATS_LIMIT) else {
        return choices;
    };
    let library = read::<usize>(ALL_PARTS);
    let Some(vtable) = diag::rd::<usize>(library) else {
        return choices;
    };
    let Some(find) = diag::rd::<usize>(vtable.wrapping_add(NODE_FIND_OID)).filter(|&p| p != 0)
    else {
        return choices;
    };
    let find: unsafe extern "C" fn(usize, *const i8) -> usize = std::mem::transmute(find);
    for index in 0..count {
        let stats = read::<usize>(table + index * size_of::<usize>());
        if !diag::readable_span(stats, STATS_EXPLOSIVE + size_of::<f32>())
            || read::<u32>(stats + offset_of!(PartStats, category)) & STRATEGIC_CATEGORY == 0
        {
            continue;
        }
        let Some(oid) = native_string(stats + STATS_OID).filter(|oid| !oid.to_bytes().is_empty())
        else {
            continue;
        };
        if choices.iter().any(|choice| choice.oid == oid) {
            continue;
        }
        // SAFETY: same virtual lookup and C string input as DefinePart; we only
        // borrow the template synchronously, without cloning or modifying it.
        let part = find(library, oid.as_ptr());
        if !diag::readable_span(part, size_of::<Body>()) || read::<u64>(part + NODE_CODE) != 0xf {
            continue;
        }
        let explosive = read::<f32>(stats + STATS_EXPLOSIVE);
        let homing_type = read::<i32>(stats + STATS_HOMING_TYPE);
        let label = missile_label(&oid, explosive > NUCLEAR_YIELD);
        choices.push(Missile {
            oid,
            label,
            homing_type,
            explosive,
        });
    }
    choices
}

unsafe fn missile_label(oid: &CStr, nuclear: bool) -> CString {
    let lookup: unsafe extern "C" fn(*mut usize, *const i8) -> usize =
        std::mem::transmute(LOOKUP_LABEL);
    let destroy: unsafe extern "C" fn(*mut usize) = std::mem::transmute(STRING_DTOR);
    let mut storage = [0usize; size_of::<EscadraString>() / size_of::<usize>()];
    // SAFETY: the lookup constructs its result in aligned caller storage.
    // Copy the bytes, then destroy it with the game CRT before storage expires.
    lookup(storage.as_mut_ptr(), oid.as_ptr());
    let translated = native_string(storage.as_ptr() as usize);
    destroy(storage.as_mut_ptr());
    let text = translated
        .as_deref()
        .filter(|text| !text.to_bytes().is_empty())
        .unwrap_or(oid);
    let mut label: Vec<u8> = text
        .to_bytes()
        .iter()
        .copied()
        .filter(|byte| *byte >= b' ' && !matches!(*byte, b'{' | b'}' | b'@'))
        .collect();
    if label.is_empty() {
        return c"MISSILE TYPE".into();
    }
    if nuclear {
        label.extend(b" (NUCLEAR)");
    }
    CString::new(label).unwrap_or_else(|_| c"MISSILE TYPE".into())
}

/// Install the discrete FIGHT and SCR_MISSILE call sites. Vanilla paths replay
/// their original calls/branches. The modloader verifies the executable image.
pub(super) unsafe fn install(session: &mut PatchSession) -> Result<(), String> {
    for (site, handler, result) in [
        (0x1_4013_d7b7, match_fight as *const (), ReturnType::Rax),
        (0x1_4013_e05e, load_fight as *const (), ReturnType::None),
        (
            0x1_402c_4dfc,
            spawn_defenders as *const (),
            ReturnType::None,
        ),
        (0x1_402d_4e80, arm_missile as *const (), ReturnType::None),
        (
            0x1_4002_bd72,
            nuclear_map_impact as *const (),
            ReturnType::None,
        ),
        (0x1_402c_5102, exit_missile as *const (), ReturnType::None),
    ] {
        let mut call = Trampoline::new();
        call.preserved_call(handler, &[], &[], result);
        call.absolute_jump(site + 5);
        session
            .detour_trampoline(site, 5, call)
            .map_err(|err| err.to_string())?;
    }

    // The normal enemy generator dereferences TEST's size checkboxes. LAUNCH is
    // also valid without opening TEST. Its roster is supplied at ENEMY_APPEND_SEAM.
    let mut generated = Trampoline::new();
    let skip = generated.new_label();
    generated.preserved_call(has_request as *const (), &[], &[], ReturnType::Rax);
    generated.bytes(&[0x84, 0xc0]); // TEST AL,AL
    generated.jump_if(Condition::NotEqual, skip);
    generated.bytes(&[0x48, 0xb9]); // LEA RCX,[native difficulty name] -> MOV RCX,imm64
    generated.bytes(&0x1_4035_c9c8u64.to_le_bytes());
    generated.absolute_jump(0x1_4013_dcf8);
    generated.bind(skip).map_err(|err| err.to_string())?;
    generated.absolute_jump(0x1_4013_df38);
    session
        .detour_trampoline(0x1_4013_dcf1, 7, generated)
        .map_err(|err| err.to_string())?;

    // For a custom encounter don't sort by AA score: the editor ship must be
    // first (and so player-controlled), even when a wingman has a higher score.
    let mut sort = Trampoline::new();
    let ordered = sort.new_label();
    sort.preserved_call(is_active as *const (), &[], &[], ReturnType::Rax);
    sort.bytes(&[0x84, 0xc0]); // TEST AL,AL
    sort.jump_if(Condition::NotEqual, ordered);
    sort.bytes(&[0x48, 0x8b, 0xc6, 0x48, 0x85, 0xf6]); // MOV RAX,RSI; TEST RSI,RSI
    sort.absolute_jump(0x1_402d_4908);
    sort.bind(ordered).map_err(|err| err.to_string())?;
    sort.absolute_jump(0x1_402d_4a0d);
    session
        .detour_trampoline(0x1_402d_4902, 6, sort)
        .map_err(|err| err.to_string())?;

    // Force the native airborne/landed initialization for our defenders,
    // otherwise keep R13B as selected by the campaign fleet's altitude.
    let mut start = Trampoline::new();
    start.preserved_call(
        defender_start as *const (),
        &[0x41, 0x0f, 0xb6, 0xcd], // MOVZX ECX,R13B
        &[],
        ReturnType::Rax,
    );
    start.bytes(&[0x41, 0x88, 0xc5]); // MOV R13B,AL; RAX is overwritten next
    start.bytes(&[0x48, 0xb9]); // MOV RCX,imm64
    start.bytes(&HGE.to_le_bytes());
    start.bytes(&[0x48, 0x8b, 0x09]); // MOV RCX,[RCX]
    start.absolute_jump(0x1_402d_46a8);
    session
        .detour_trampoline(0x1_402d_46a1, 7, start)
        .map_err(|err| err.to_string())?;

    // Extend the vanilla four-ship loop only in Shipworks. Adjust the
    // positions of landed and extra airborne ships before the native clamp.
    let mut position = Trampoline::new();
    position.bytes(&[0xf2, 0x0f, 0x11, 0x45, 0x67]); // MOVSD [RBP+0x67],XMM0
    position.preserved_call(
        place_defender as *const (),
        &[0x48, 0x8d, 0x4d, 0x67, 0x8b, 0xd7], // LEA RCX,[RBP+0x67]; MOV EDX,EDI
        &[],
        ReturnType::None,
    );
    position.absolute_jump(0x1_402d_4b26);
    session
        .detour_trampoline(0x1_402d_4b21, 5, position)
        .map_err(|err| err.to_string())?;

    let mut limit = Trampoline::new();
    let full = limit.new_label();
    limit.preserved_call(
        at_defender_limit as *const (),
        &[0x8b, 0xcf], // MOV ECX,EDI
        &[],
        ReturnType::Rax,
    );
    limit.bytes(&[0x84, 0xc0]); // TEST AL,AL
    limit.jump_if(Condition::NotEqual, full);
    limit.absolute_jump(0x1_402d_4c83);
    limit.bind(full).map_err(|err| err.to_string())?;
    limit.absolute_jump(0x1_402d_4c8f);
    session
        .detour_trampoline(0x1_402d_4c7e, 5, limit)
        .map_err(|err| err.to_string())?;

    // Body_Explode's kind-4 branch ends after the nuke effects. The handler
    // returns a nonzero f32 bit pattern only when we should also run the
    // ordinary tactical explosion continuation (XMM1 is its yield input).
    // XMM10 (read as zero by the continuation) and R14 (the body) are
    // nonvolatile in the Windows x64 ABI, so both callbacks preserve them.
    let mut nuke = Trampoline::new();
    let effects_only = nuke.new_label();
    nuke.preserved_call(nuclear_effect as *const (), &[], &[], ReturnType::Rax);
    nuke.bytes(&[0x85, 0xc0]); // TEST EAX,EAX
    nuke.jump_if(Condition::Equal, effects_only);
    nuke.bytes(&[0x66, 0x0f, 0x6e, 0xc8]); // MOVD XMM1,EAX
    nuke.absolute_jump(0x1_4002_9c0c);
    nuke.bind(effects_only).map_err(|err| err.to_string())?;
    nuke.bytes(&[0xb0, 0x01]); // Native nuke effects always return AL=1.
    nuke.absolute_jump(0x1_4002_9c07);
    session
        .detour_trampoline(0x1_4002_9c02, 5, nuke)
        .map_err(|err| err.to_string())?;

    // A nuke enables a late battle pass that applies the map's CRT shader
    // to the entire frame. Keep the effect but choose ps_mainNoFishEye.
    let mut render = Trampoline::new();
    render.preserved_call(
        battle_screen_technique as *const (),
        &[],
        &[],
        ReturnType::Rax,
    );
    render.bytes(&[0x48, 0x89, 0xc2, 0x48, 0x8b, 0x01]); // MOV RDX,RAX; MOV RAX,[RCX]
    render.absolute_jump(0x1_4030_9b9e);
    session
        .detour_trampoline(0x1_4030_9b94, 10, render)
        .map_err(|err| err.to_string())?;
    Ok(())
}

unsafe extern "C" fn has_request() -> bool {
    requested()
}

unsafe extern "C" fn is_active() -> bool {
    active()
}

unsafe extern "C" fn match_fight(name: usize, expected: *const i8) -> u8 {
    // SAFETY: these are the live native string and literal from the original
    // comparison. Only our pre-dispatch-approved button may alias the literal.
    if requested() && native_string(name).is_some_and(|name| name.as_c_str() == LAUNCH_NAME) {
        return 1;
    }
    let matches: unsafe extern "C" fn(usize, *const i8) -> u8 = std::mem::transmute(MATCH_NAME);
    matches(name, expected)
}

type LoadLevel = unsafe extern "C" fn(*const i8, u8, u8);

unsafe extern "C" fn load_fight(path: *const i8, resume: u8, flag: u8) {
    let request = REQUEST
        .try_lock()
        .ok()
        .and_then(|mut request| request.take());
    let load: LoadLevel = std::mem::transmute(LOAD_LEVEL);
    let Some(launch) = request else {
        load(path, resume, flag);
        return;
    };
    let _hook = diag::scope("Shipworks missile staging");
    match stage_defenders(launch.roster) {
        Ok(()) => {
            log::info!(
                "hf_ally: Shipworks Missile Defense launch {:?} / {:?}: {}",
                launch.roster,
                launch.start,
                launch.missile.oid.to_string_lossy()
            );
            if let Ok(mut battle) = BATTLE.try_lock() {
                *battle = Some(launch);
                ACTIVE.store(true, Ordering::Relaxed);
                drop(battle);
                load(MISSILE_LEVEL.as_ptr(), resume, flag);
                return;
            }
        }
        Err(reason) => log::warn!("hf_ally: Shipworks Missile Defense could not launch: {reason}"),
    }
    prune_enemy_escadra();
    load(EDITOR_LEVEL.as_ptr(), 0, 0);
}

/// Both side vectors have just been built by FIGHT. Side A is the defender
/// list. Side B stays intact until SCR_MISSILE state 1 replaces its first link.
unsafe fn stage_defenders(roster: Roster) -> Result<(), &'static str> {
    let ally_link = single_link(FRIENDLY_LIST).ok_or("missing editor fleet link")?;
    let enemy_link = single_link(ENEMY_LIST).ok_or("missing test enemy fleet link")?;
    let allies = read::<usize>(ally_link + LINK_TARGET);
    let enemies = read::<usize>(enemy_link + LINK_TARGET);
    if diag::rd::<u64>(allies.wrapping_add(NODE_CODE)) != Some(ESCADRA_CODE)
        || diag::rd::<u64>(enemies.wrapping_add(NODE_CODE)) != Some(ESCADRA_CODE)
    {
        return Err("invalid Shipworks fleet");
    }
    if matches!(roster, Roster::Strike | Roster::Mixed)
        && !diag::rd::<usize>(
            enemies + offset_of!(Node, children) + offset_of!(SentinelList<usize>, length),
        )
        .is_some_and(|count| (1..=ENEMY_LIMIT).contains(&count))
    {
        return Err("no enemy snapshot could be loaded");
    }
    // SAFETY: both Links are initialized, live FIGHT vector elements. Native
    // reassignment/copy keeps their registrations and owns any vector growth.
    match roster {
        Roster::Strike => {
            let set: SetLinkFn = std::mem::transmute(SET_LINK);
            set(ally_link, enemies);
        }
        Roster::Mixed => {
            let push: unsafe extern "C" fn(usize, usize) = std::mem::transmute(PUSH_LINK);
            push(FRIENDLY_LIST, enemy_link);
        }
        Roster::Defense | Roster::AlliedDefense => {}
    }
    Ok(())
}

unsafe fn single_link(vector: usize) -> Option<usize> {
    let (begin, count) = vector_span(vector, LINK_NODE_STRIDE, 1)?;
    (count == 1).then_some(begin)
}

unsafe extern "C" fn spawn_defenders() {
    let original: unsafe extern "C" fn() = std::mem::transmute(SPAWN_DEFENDERS);
    if !active() {
        original();
        return;
    }
    let _hook = diag::scope("Shipworks missile scene spawn");
    // The native spawner unconditionally dereferences B[0]. Only enter it
    // after a native Missile has been constructed and registered there.
    let launch = BATTLE.try_lock().ok().and_then(|mut battle| battle.take());
    let prepared = launch
        .as_ref()
        .is_some_and(|launch| prepare_missile(launch));
    if !prepared {
        log::warn!(
            "hf_ally: Shipworks Missile Defense missing its staged missile; returning to editor"
        );
        let load: LoadLevel = std::mem::transmute(LOAD_LEVEL);
        load(EDITOR_LEVEL.as_ptr(), 0, 0);
        return;
    }
    write(TEST_FIGHT_FLAG, 1u8);
    original();
}

unsafe fn prepare_missile(launch: &Launch) -> bool {
    let Some(enemy_link) = single_link(ENEMY_LIST) else {
        return false;
    };
    let Some(defenders) = staged_defenders() else {
        return false;
    };
    let incoming = launch.roster.missile_side();
    if !defenders
        .first()
        .is_some_and(|&ship| read::<f32>(ship + OFF_SIDE) * incoming < 0.0)
        || OWN_MISSILE.load(Ordering::Relaxed) != 0
    {
        return false;
    }
    let allocate: unsafe extern "C" fn(usize) -> usize = std::mem::transmute(NEW);
    let construct: unsafe extern "C" fn(usize) -> usize = std::mem::transmute(MISSILE_CTOR);
    let memory = allocate(MISSILE_SIZE);
    if memory == 0 {
        return false;
    }
    // SAFETY: this allocation has the native size and allocator. The constructor
    // initializes the OID and native links; the missile is never added to a profile.
    let missile = construct(memory);
    let copy: unsafe extern "C" fn(usize, *const i8, usize) -> usize =
        std::mem::transmute(super::COPY_STRING);
    copy(
        missile + MAP_MISSILE_OID,
        launch.missile.oid.as_ptr(),
        launch.missile.oid.to_bytes().len(),
    );
    write(missile + MAP_MISSILE_COUNT, 1i32);
    write(
        missile + MAP_MISSILE_HOMING_TYPE,
        launch.missile.homing_type,
    );
    write(missile + MAP_MISSILE_EXPLOSIVE, launch.missile.explosive);
    // The constructor leaves map position and target position at the same
    // point. Logic 10 therefore takes the native terminal homing path.
    write(missile + MAP_SIDE, incoming);
    let set: SetLinkFn = std::mem::transmute(SET_LINK);
    set(enemy_link, missile);
    OWN_MISSILE.store(missile, Ordering::Relaxed);
    GROUNDED_DEFENDERS.store(
        match launch.start {
            DefenderStart::Airborne => 0,
            DefenderStart::Ground => defenders.len(),
        },
        Ordering::Relaxed,
    );
    for (index, ship) in defenders.into_iter().enumerate() {
        // World-list insertion reads this flag. Editor snapshots may still
        // carry the player flag; only the first friendly defender may keep it.
        write(ship + OFF_AI_FLAG, u8::from(index != 0 || incoming > 0.0));
    }
    true
}

/// Walk current staged Escadras with the same order and eligibility check as
/// the native defender spawner. Pointers are borrowed within state 1 only.
unsafe fn staged_defenders() -> Option<Vec<usize>> {
    let (begin, fleet_count) = vector_span(FRIENDLY_LIST, LINK_NODE_STRIDE, 2)?;
    let get_ship: unsafe extern "C" fn(usize) -> usize = std::mem::transmute(PROPERTY_SHIP);
    let has_thrust: unsafe extern "C" fn(usize) -> u8 = std::mem::transmute(HAS_THRUST);
    let mut ships = Vec::new();
    for index in 0..fleet_count {
        let fleet = read::<usize>(begin + index * LINK_NODE_STRIDE + LINK_TARGET);
        if !diag::readable_span(fleet, size_of::<Node>())
            || read::<u64>(fleet + NODE_CODE) != ESCADRA_CODE
        {
            return None;
        }
        let children = fleet + offset_of!(Node, children);
        let head = read::<usize>(children);
        let count = read::<usize>(children + offset_of!(SentinelList<usize>, length));
        if count > CUSTOM_DEFENDER_LIMIT {
            return None;
        }
        let mut cell = diag::rd::<usize>(head)?;
        for _ in 0..count {
            if cell == head {
                return None;
            }
            let property =
                diag::rd::<usize>(cell.checked_add(offset_of!(SentinelListNode<usize>, value))?)?;
            if !diag::readable_span(property, size_of::<Node>()) {
                return None;
            }
            // SAFETY: these native property nodes and linked templates belong
            // to the freshly staged Escadra, on the scene's game thread.
            let ship = get_ship(property);
            if diag::readable_span(ship, size_of::<Creature>())
                && read::<u64>(ship + NODE_CODE) == CREATURE_CODE
                && has_thrust(ship) != 0
            {
                ships.push(ship);
            }
            cell = diag::rd::<usize>(cell)?;
        }
        if cell != head {
            return None;
        }
    }
    (ships.len() <= CUSTOM_DEFENDER_LIMIT).then_some(ships)
}

unsafe extern "C" fn defender_start(native_airborne: u8) -> u8 {
    if active() {
        u8::from(GROUNDED_DEFENDERS.load(Ordering::Relaxed) == 0)
    } else {
        native_airborne
    }
}

unsafe extern "C" fn place_defender(position: usize, index: u32) {
    if !active() {
        return;
    }
    let grounded = GROUNDED_DEFENDERS.load(Ordering::Relaxed);
    if grounded == 0 && index < VANILLA_DEFENDER_LIMIT {
        return;
    }
    let right = read::<f32>(ARENA_RIGHT);
    let left = read::<f32>(ARENA_LEFT);
    let span = right - left;
    let bottom = read::<f32>(ARENA_BOTTOM);
    if !span.is_finite() || span <= 2.0 * GROUND_EDGE_MARGIN || !bottom.is_finite() {
        return;
    }
    let from_right = read::<u8>(FROM_RIGHT) != 0;
    let (inset, y) = if grounded != 0 {
        // One row from the native entry side inward, nearest ship first. The
        // supplied Y is below the generated surface; native ground-height
        // and hull-height code clamps it to the actual landing position.
        let gaps = grounded.saturating_sub(1) as f32;
        let spacing = ((span - 2.0 * GROUND_EDGE_MARGIN) / gaps.max(1.0)).min(GROUND_MAX_SPACING);
        (GROUND_EDGE_MARGIN + index as f32 * spacing, bottom)
    } else {
        // The four native slots occupy the entry-side line, so put extras on
        // the opposite half, stacked in rows that stay inside the arena.
        let extra = index - VANILLA_DEFENDER_LIMIT;
        let column = extra % EXTRA_DEFENDERS_PER_ROW;
        let row = extra / EXTRA_DEFENDERS_PER_ROW;
        (
            span * 0.12 + column as f32 * span * 0.07,
            EXTRA_DEFENDER_HEIGHT - row as f32 * EXTRA_ROW_HEIGHT,
        )
    };
    let x = if grounded != 0 {
        // Landed ships start at the flag-selected edge and continue inward.
        if from_right {
            right - inset
        } else {
            left + inset
        }
    } else if from_right {
        left + inset
    } else {
        right - inset
    };
    // SAFETY: at this seam RBP+0x67 is a writable, possibly unaligned
    // game-stack Vec2, still to be clamped against terrain by the native path.
    write(position, x);
    write(position + size_of::<f32>(), y);
}

unsafe extern "C" fn at_defender_limit(index: u32) -> bool {
    let limit = if active() {
        CUSTOM_DEFENDER_LIMIT as u32
    } else {
        VANILLA_DEFENDER_LIMIT
    };
    index >= limit
}

// SAFETY: caller verified `ship` spans at least `size_of::<Creature>()` readable bytes.
// `CREATURE_RADAR` (+0x17a0, `m_tele_radar`) and `CREATURE_JAMMER` (+0x17c4, `m_tele_jammer`)
// are plain `f32` stats populated by `Creature_AccumulatePartStats` (0x14007229b / 0x140072592)
// only from live, powered radar/FCR and jammer parts.
unsafe fn radiation_emission(ship: usize) -> f32 {
    let radar = read::<f32>(ship + CREATURE_RADAR);
    let jammer = read::<f32>(ship + CREATURE_JAMMER);
    let radar = if radar.is_finite() && radar > 0.0 {
        radar
    } else {
        0.0
    };
    let jammer = if jammer.is_finite() && jammer > 0.0 {
        jammer
    } else {
        0.0
    };
    radar + jammer
}

unsafe extern "C" fn arm_missile(body: usize, map_missile: usize) {
    if map_missile == 0 || map_missile != OWN_MISSILE.load(Ordering::Relaxed) {
        let assign: unsafe extern "C" fn(usize, usize) = std::mem::transmute(ASSIGN_DEFENDER);
        assign(body, map_missile);
        return;
    }
    let _hook = diag::scope("Shipworks missile target");
    // SAFETY: the native spawner just defined/attached this Body and registered
    // it in the missile vector. Body Links are initialized native objects.
    let side = read::<f32>(map_missile + MAP_SIDE);
    let homing_type = read::<i32>(map_missile + MAP_MISSILE_HOMING_TYPE);
    let conventional = native_string(body + offset_of!(Body, oid))
        .is_some_and(|oid| oid.to_bytes().ends_with(b"_CONV"));
    let set: SetLinkFn = std::mem::transmute(SET_LINK);
    let (target, emission) = vector_span(ROSTER_BEGIN, LINK_NODE_STRIDE, CUSTOM_DEFENDER_LIMIT)
        .and_then(|(begin, count)| {
            if homing_type == HOMING_ANTI_RADIATION {
                // Anti-radiation ("P") missiles only home on radar/jammer emitters.
                // Pick the strongest opposing emitter, matching map seeker 0x14010f383.
                let mut best: Option<(usize, f32)> = None;
                for index in 0..count {
                    let ship = read::<usize>(begin + index * LINK_NODE_STRIDE + LINK_TARGET);
                    if !diag::readable_span(ship, size_of::<Creature>())
                        || read::<f32>(ship + OFF_SIDE) * side >= 0.0
                    {
                        continue;
                    }
                    let strength = radiation_emission(ship);
                    if strength > 0.0 && best.is_none_or(|(_, prev)| strength > prev) {
                        best = Some((ship, strength));
                    }
                }
                best
            } else {
                (0..count).find_map(|index| {
                    let ship = read::<usize>(begin + index * LINK_NODE_STRIDE + LINK_TARGET);
                    (diag::readable_span(ship, size_of::<Creature>())
                        && read::<f32>(ship + OFF_SIDE) * side < 0.0)
                        .then_some((ship, 0.0))
                })
            }
        })
        .unwrap_or((0, 0.0));
    write(body + TACTICAL_MISSILE_SIDE, side);
    // When no target is acquired (e.g. a P missile with no radar/jammer emitter),
    // match SCR_MISSILE_AssignDefender's fly-past state (0x140263700): logic 10
    // with link_2 = map_missile and link_1 = 0 flies straight away, whereas
    // logic 1 with link_1 = 0 would immediately enter its 4 s tumble/self-destruct.
    let fly_past = target == 0;
    write(
        body + offset_of!(Body, logic),
        if conventional && !fly_past {
            1i32
        } else {
            10i32
        },
    );
    write(body + offset_of!(Body, logic_active), 1u8);
    set(
        body + offset_of!(Body, projectile_link_2),
        if conventional && !fly_past {
            0
        } else {
            map_missile
        },
    );
    set(body + offset_of!(Body, projectile_link_1), target);
    if fly_past && homing_type == HOMING_ANTI_RADIATION {
        log::info!(
            "hf_ally: Shipworks P missile {body:#x} found no radar/jammer emitter; flying past (side {side:+})"
        );
    } else {
        log::info!(
            "hf_ally: Shipworks missile {body:#x} armed against {target:#x} (side {side:+}, homing {homing_type}, emission {emission:.1})"
        );
    }
    diag::flush_log();
}

/// Body_Explode's kind-4 branch normally stops after the strategic/visual nuke
/// handler. In the Shipworks sandbox, run that handler and report the clamped
/// yield so the trampoline can continue into the ordinary tactical blast.
unsafe extern "C" fn nuclear_effect(body: usize) -> u32 {
    if !active() {
        // SAFETY: the native branch must run exactly once for every nuke body.
        let effects: unsafe extern "C" fn(usize) = std::mem::transmute(NUCLEAR_EFFECT);
        effects(body);
        return 0;
    }
    // Read the yield before the native handler replaces the detonating body
    // with its own visual explosion.
    // SAFETY: R14/RCX in Body_Explode is the live kind-4 Body; its explosive
    // power is a plain scalar at the documented layout offset.
    let yield_power =
        diag::rd::<f32>(body.wrapping_add(offset_of!(Body, explosive_power))).unwrap_or(0.0);
    let clamped = if yield_power.is_finite() && yield_power > MAX_TACTICAL_NUCLEAR_YIELD {
        MAX_TACTICAL_NUCLEAR_YIELD
    } else {
        yield_power
    };
    // SAFETY: the native branch must run exactly once for every nuke body.
    let effects: unsafe extern "C" fn(usize) = std::mem::transmute(NUCLEAR_EFFECT);
    effects(body);
    clamped.to_bits()
}

/// The strategic impact handler mutates the campaign Profile and the staged
/// escadra. A Shipworks test must not do that; its battle scene keeps its own
/// visual and tactical blast. Replay the native call outside our scene.
unsafe extern "C" fn nuclear_map_impact(target: usize, scale: f32, angle: f32) {
    if active() {
        return;
    }
    let impact: unsafe extern "C" fn(usize, f32, f32) = std::mem::transmute(NUCLEAR_MAP_IMPACT);
    impact(target, scale, angle);
}

/// The late battle pass that a nuke enables uses ShaderMapPostProcessing's
/// warped `ps_main`, which pulls the whole frame through the map CRT trapezoid.
/// Keep the pass, but select the stock `ps_mainNoFishEye` technique instead.
unsafe extern "C" fn battle_screen_technique(shader: usize) -> usize {
    let technique = if active() {
        SHADER_UNWARPED_TECHNIQUE
    } else {
        SHADER_MAP_TECHNIQUE
    };
    diag::rd::<usize>(shader.wrapping_add(technique))
        .filter(|&pointer| pointer != 0)
        .or_else(|| diag::rd::<usize>(shader.wrapping_add(SHADER_MAP_TECHNIQUE)))
        .unwrap_or(0)
}

unsafe extern "C" fn exit_missile(path: *const i8, resume: u8, flag: u8) {
    let load: LoadLevel = std::mem::transmute(LOAD_LEVEL);
    load(
        if active() {
            EDITOR_LEVEL.as_ptr()
        } else {
            path
        },
        resume,
        flag,
    );
}

/// The next level is not Missile.seria. No more custom scene ticks should run.
/// Called on the game thread before the native loader copies the new path.
pub(super) unsafe fn leave() {
    cancel_request();
    if !ACTIVE.swap(false, Ordering::Relaxed) {
        return;
    }
    if let Ok(mut battle) = BATTLE.try_lock() {
        *battle = None;
    }
    let marker = OWN_MISSILE.swap(0, Ordering::Relaxed);
    let missile = single_link(ENEMY_LIST)
        .map(|link| read::<usize>(link + LINK_TARGET))
        .filter(|&missile| {
            missile == marker
                && diag::rd::<u64>(missile.wrapping_add(NODE_CODE)) == Some(MISSILE_CODE)
        });
    // SAFETY: these are the native Link vectors staged by the test FIGHT.
    // Clearing destroys the Links, not the targets. The native
    // Missile destructor unregisters any remaining references to our object.
    let clear: unsafe extern "C" fn(usize) = std::mem::transmute(LINK_CLEAR_VECTOR);
    clear(FRIENDLY_LIST);
    clear(ENEMY_LIST);
    if let Some(missile) = missile {
        let destroy: unsafe extern "C" fn(usize, usize) -> usize =
            std::mem::transmute(MISSILE_DTOR);
        destroy(missile, 1);
    }
    prune_enemy_escadra();
    log::info!("hf_ally: Shipworks Missile Defense ended; cleared staging links");
}
