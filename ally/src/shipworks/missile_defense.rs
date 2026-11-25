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
    native_layout::{
        Body, Creature, EscadraString, Node, PartStats, SentinelList, NODE_CODE, NODE_OWNER,
    },
    precombat, sound,
    time::now_ms,
};

use super::{
    damage::{self, Replay, Slot},
    design, native_string, prune_enemy_escadra, show_control, write, ControlGeometry, BUTTON_STYLE,
    CLICK_DEBOUNCE_MS, ENEMY_LIMIT, ENEMY_LIST, LINK_CLEAR_VECTOR, LINK_NODE_STRIDE, LINK_TARGET,
    ROSTER_LIMIT, ROW_STYLE, SESSION, SIDE_ALLY, SIDE_ENEMY, TEST_FIGHT_FLAG,
};

const TYPE_NAME: &CStr = c"SW_ACE_MSL";
const START_NAME: &CStr = c"SW_ACE_START";
const LAUNCH_NAME: &CStr = c"SW_ACE_LAUNCH";
const PRESERVE_NAME: &CStr = c"SW_ACE_KEEP";
const VIEW_NAME: &CStr = c"SW_ACE_DMG";
const PREV_NAME: &CStr = c"SW_DMG_PREV";
const NEXT_NAME: &CStr = c"SW_DMG_NEXT";
const PAGE_NAME: &CStr = c"SW_DMG_PAGE";
const MISSILE_LEVEL: &CStr = c"Missile.seria";
const EDITOR_LEVEL: &CStr = c"Editor.seria";
const FRAME_WIDTH: usize = 0x1_43a1_58fc;
const FRAME_HEIGHT: usize = 0x1_43a1_3e5c;
const SCHEMATIC_HEIGHT: f32 = 220.0;
const PANEL_RIGHT_MARGIN: f32 = 36.0;
const PANEL_TOP_GAP: f32 = 32.0;
const PANEL_ROW_GAP: f32 = 8.0;
static TYPE_GUI: AtomicUsize = AtomicUsize::new(0);
static START_GUI: AtomicUsize = AtomicUsize::new(0);
static LAUNCH_GUI: AtomicUsize = AtomicUsize::new(0);
static PRESERVE_GUI: AtomicUsize = AtomicUsize::new(0);
static VIEW_GUI: AtomicUsize = AtomicUsize::new(0);
static PREV_GUI: AtomicUsize = AtomicUsize::new(0);
static NEXT_GUI: AtomicUsize = AtomicUsize::new(0);
static PAGE_GUI: AtomicUsize = AtomicUsize::new(0);

const STATS_TABLE: usize = 0x1_43a3_3bf0;
const STATS_LIMIT: usize = 4096;
// Stats_GetIndexMap (0x140281fa0) uses the first opaque EscadraString as the OID.
const STATS_OID: usize = offset_of!(PartStats, unknown_0048h);
const STATS_HOMING_TYPE: usize = 0x164;
const STATS_EXPLOSIVE: usize = 0x168;
const STRATEGIC_CATEGORY: u32 = 0x0800_0000;
const NUCLEAR_YIELD: f32 = 100_000.0;
// Campaign measurement (v1.3.29): a strategic nuke hit cost each defending ship
// about 1.8k-2.9k hull and 11-24 destroyed parts, i.e. a concentrated hit on a
// limited set of parts, not a uniform area blast. Spend a per-ship HP budget
// instead of a fixed number of damage calls: destroying the nearest parts at
// their actual HP avoids overkill, and the budget scales with the ship's part
// count so a 533-part Sevastopol takes a nuke-sized hit, not 16 small parts.
const SANDBOX_NUKE_DAMAGE_PER_MORAL: f32 = 2_500.0;
const SANDBOX_NUKE_REFERENCE_PARTS: f32 = 381.0;
// Campaign strategic blast intensity (Explosion::Update 0x140116040 ->
// 0x1400e6aa0 -> Math_InverseCubeFalloffCapped 0x1402123e0):
//   min(cap, E / (1e6 * d^3))
// with cap 999 for an AI ship (alignment <= 0) and 50 for a player ship, and
// the distance floored at 1.0 / 2.0 respectively. At E=4e7, d=1 this is 40
// damage per part; the measured 2,500-hull close hit is 62.5 such parts. The
// exact per-part bounding-box distribution inside 0x1400e6aa0 is not decoded,
// so the evenly spaced hull sample below remains the measured stand-in.
const CAMPAIGN_NUKE_FALLOFF_DIVISOR: f32 = 1_000_000.0;
const CAMPAIGN_NUKE_AI_CAP: f32 = 999.0;
const CAMPAIGN_NUKE_PLAYER_CAP: f32 = 50.0;
const CAMPAIGN_NUKE_REFERENCE_DAMAGE: f32 = 40.0;
const CAMPAIGN_NUKE_DISTANCE_FLOOR_AI: f32 = 1.0;
const CAMPAIGN_NUKE_DISTANCE_FLOOR_PLAYER: f32 = 2.0;
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
const TEARDOWN: usize = 0x1_402d_4f20;
const ASSIGN_DEFENDER: usize = 0x1_4026_3700;
const BROADCAST_GEAR_COMMAND: usize = 0x1_4007_9b20;
const SNAP_FOLDED_GEAR: usize = 0x1_401b_ccb0;
const AIRBORNE_GEAR_SITE: usize = 0x1_402d_4bdf;
const HGE: u64 = 0x1_43a1_19e0;
const PROPERTY_SHIP: usize = 0x1_4021_b070;
const HAS_THRUST: usize = 0x1_4007_3450;
const VANILLA_DEFENDER_LIMIT: u32 = 4;
pub(super) const CUSTOM_DEFENDER_LIMIT: usize = 1 + ROSTER_LIMIT + ENEMY_LIMIT;
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
// Campaign diagnostics only: the profile owns the created Explosion marks.
const GET_PROFILE: usize = 0x1_4029_2a90;
const PROFILE_ESCALATION: usize = 0x2ec;
const PROFILE_EXPLOSIONS: usize = 0x240;
const EXPLOSION_POWER: usize = 0x24c;
const EXPLOSION_RADIUS: usize = 0x254;
const EXPLOSION_YIELD: usize = 0x258;
const EXPLOSION_IS_NUCLEAR: usize = 0x260;
const EXPLOSION_SHIP_DAMAGED: usize = 0x261;
const MARK_POSITION_X: usize = 0x150;
const MARK_POSITION_Y: usize = 0x154;
const MARK_ALTITUDE: usize = 0x168;
// Kind-4 Body field that links back to the staged map Missile.
const MAP_MISSILE_LINK: usize = 0x4b0;
const NUCLEAR_EXIT_BRANCH: usize = 0x1_402c_5067;
const NUCLEAR_EXIT_STATE_10: usize = 0x1_402b_e866;
const MISSILE_EXIT_COUNTDOWN: usize = 0x1_402c_506d;
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
    preserve: bool,
    replay: Vec<(Slot, Replay)>,
    staged: Vec<(usize, Slot)>,
}

impl Launch {
    fn replay_for(&self, slot: Slot) -> Option<&Replay> {
        self.replay
            .iter()
            .find(|(key, _)| *key == slot)
            .map(|(_, state)| state)
    }
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
    preserve: bool,
    page: Option<usize>,
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
            preserve: false,
            page: None,
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
        self.page = None;
        damage::preview(None);
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
        let step = height + PANEL_ROW_GAP;
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
                row(y + step),
                BUTTON_STYLE,
            );
            show_control(
                PRESERVE_NAME.to_bytes_with_nul(),
                &PRESERVE_GUI,
                listed,
                template,
                if self.preserve {
                    c"[X] PRESERVE DAMAGE"
                } else {
                    c"[ ] PRESERVE DAMAGE"
                },
                row(y + 2.0 * step),
                BUTTON_STYLE,
            );
            show_control(
                LAUNCH_NAME.to_bytes_with_nul(),
                &LAUNCH_GUI,
                listed,
                template,
                c"LAUNCH",
                row(y + 3.0 * step),
                BUTTON_STYLE,
            );
        } else {
            hide_button(listed, START_NAME, &START_GUI);
            hide_button(listed, LAUNCH_NAME, &LAUNCH_GUI);
            hide_button(listed, PRESERVE_NAME, &PRESERVE_GUI);
        }
        show_control(
            VIEW_NAME.to_bytes_with_nul(),
            &VIEW_GUI,
            listed,
            template,
            c"DMG VIEW",
            row(y + 4.0 * step),
            BUTTON_STYLE,
        );
        let Some(page_index) = self.page else {
            hide_viewer(listed);
            return;
        };
        let (count, page) = damage::pages(page_index);
        let header_y = y + 5.0 * step;
        let mut title = Vec::new();
        if let Some(page) = &page {
            title.extend_from_slice(format!("{}/{}  ", page_index + 1, count).as_bytes());
            title.extend_from_slice(page.name.to_bytes());
            if page.destroyed {
                title.extend_from_slice(b" (DESTROYED)");
            }
            if !page.has_panel() {
                title.extend_from_slice(b" - NO PANEL CAPTURED");
            }
        }
        let title = if title.is_empty() {
            c"NO COMBAT RESULTS".into()
        } else {
            CString::new(title).unwrap_or_else(|_| c"COMBAT RESULTS".into())
        };
        show_control(
            PAGE_NAME.to_bytes_with_nul(),
            &PAGE_GUI,
            listed,
            template,
            &title,
            ControlGeometry {
                position: [x - panel_width * 0.5, header_y],
                dimensions: [panel_width, height],
            },
            ROW_STYLE,
        );
        let schematic_y = header_y + height * 0.5 + PANEL_ROW_GAP;
        let nav_y = schematic_y + SCHEMATIC_HEIGHT + PANEL_ROW_GAP + height * 0.5;
        for (name, gui, label, offset) in [
            (PREV_NAME, &PREV_GUI, c"PREV", -panel_width * 0.25),
            (NEXT_NAME, &NEXT_GUI, c"NEXT", panel_width * 0.25),
        ] {
            show_control(
                name.to_bytes_with_nul(),
                gui,
                listed,
                template,
                label,
                ControlGeometry {
                    position: [x + offset, nav_y],
                    dimensions: [panel_width * 0.5 - PANEL_ROW_GAP, height],
                },
                BUTTON_STYLE,
            );
        }
        let frame_height = read::<i32>(FRAME_HEIGHT) as f32;
        damage::preview(page.map(|page| {
            (
                ControlGeometry {
                    position: [
                        x + frame_width * 0.5,
                        schematic_y + SCHEMATIC_HEIGHT * 0.5 + frame_height * 0.5,
                    ],
                    dimensions: [panel_width, SCHEMATIC_HEIGHT],
                },
                page,
            )
        }));
    }

    /// True only when the native dispatcher should run its test-fight branch.
    pub(super) unsafe fn click(&mut self, name: &str, allies: bool, enemies: bool) -> bool {
        if !self.open || active() {
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
        if id == VIEW_NAME {
            self.page = if self.page.is_some() { None } else { Some(0) };
            sound::button("Shipworks damage viewer");
            return false;
        }
        if id == PREV_NAME || id == NEXT_NAME {
            if let Some(index) = self.page {
                let (count, _) = damage::pages(index);
                if count > 1 {
                    self.page = Some(if id == PREV_NAME {
                        (index + count - 1) % count
                    } else {
                        (index + 1) % count
                    });
                    sound::button("Shipworks damage page");
                }
            }
            return false;
        }
        if self.choices.is_empty() {
            return false;
        }
        if id == PRESERVE_NAME {
            self.preserve = !self.preserve;
            sound::button("Shipworks preserve damage");
            return false;
        }
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
        let replay = if self.preserve {
            damage::replay()
        } else {
            Vec::new()
        };
        let saved = replay
            .iter()
            .filter(|(_, state)| matches!(state, damage::Replay::Saved(_)))
            .count();
        let destroyed = replay
            .iter()
            .filter(|(_, state)| matches!(state, damage::Replay::Destroyed))
            .count();
        *request = Some(Launch {
            missile,
            roster,
            start: self.start,
            preserve: self.preserve,
            replay,
            staged: Vec::new(),
        });
        drop(request);
        log::info!("hf_ally: Shipworks preserve replay: {saved} saved, {destroyed} destroyed");
        self.page = None;
        damage::preview(None);
        sound::button("Shipworks missile launch");
        true
    }
}

pub(super) fn is_control(name: &str) -> bool {
    control_id(name).is_some()
}

fn control_id(name: &str) -> Option<&'static CStr> {
    [
        TYPE_NAME,
        START_NAME,
        LAUNCH_NAME,
        PRESERVE_NAME,
        VIEW_NAME,
        PREV_NAME,
        NEXT_NAME,
    ]
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
    hide_button(listed, PRESERVE_NAME, &PRESERVE_GUI);
    hide_button(listed, VIEW_NAME, &VIEW_GUI);
    hide_viewer(listed);
}

unsafe fn hide_viewer(listed: &[usize]) {
    hide_button(listed, PAGE_NAME, &PAGE_GUI);
    hide_button(listed, PREV_NAME, &PREV_GUI);
    hide_button(listed, NEXT_NAME, &NEXT_GUI);
    damage::preview(None);
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
pub(super) unsafe fn vector_span(
    header: usize,
    stride: usize,
    limit: usize,
) -> Option<(usize, usize)> {
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
            AIRBORNE_GEAR_SITE,
            set_airborne_gear as *const (),
            ReturnType::None,
        ),
        (
            0x1_4002_bd72,
            nuclear_map_impact as *const (),
            ReturnType::None,
        ),
        (0x1_402c_5102, exit_missile as *const (), ReturnType::None),
        (
            0x1_402c_50f1,
            finish_defenders as *const (),
            ReturnType::None,
        ),
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

    // In SCR_MISSILE, a finished encounter tests the nuke-effect byte
    // (0x1403c2b6b). Its alternate exit enters shared state 10 instead of
    // Missile's countdown/fade and state-100 teardown. Keep the blast, but
    // let the sandbox take the normal return path and capture its results.
    let mut exit = Trampoline::new();
    let countdown = exit.new_label();
    // Consume the native CMP flags before making the callback.
    exit.jump_if(Condition::Equal, countdown);
    exit.preserved_call(keep_missile_exit as *const (), &[], &[], ReturnType::Rax);
    exit.bytes(&[0x84, 0xc0]); // TEST AL,AL
    exit.jump_if(Condition::NotEqual, countdown);
    exit.absolute_jump(NUCLEAR_EXIT_STATE_10);
    exit.bind(countdown).map_err(|err| err.to_string())?;
    exit.absolute_jump(MISSILE_EXIT_COUNTDOWN);
    session
        .detour_trampoline(
            NUCLEAR_EXIT_BRANCH,
            MISSILE_EXIT_COUNTDOWN - NUCLEAR_EXIT_BRANCH,
            exit,
        )
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
    damage::install(session)
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
    let Some(mut launch) = request else {
        load(path, resume, flag);
        return;
    };
    let _hook = diag::scope("Shipworks missile staging");
    match stage_defenders(&mut launch) {
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
unsafe fn stage_defenders(launch: &mut Launch) -> Result<(), &'static str> {
    let ally_link = single_link(FRIENDLY_LIST).ok_or("missing editor fleet link")?;
    let enemy_link = single_link(ENEMY_LIST).ok_or("missing test enemy fleet link")?;
    let allies = read::<usize>(ally_link + LINK_TARGET);
    let enemies = read::<usize>(enemy_link + LINK_TARGET);
    if diag::rd::<u64>(allies.wrapping_add(NODE_CODE)) != Some(ESCADRA_CODE)
        || diag::rd::<u64>(enemies.wrapping_add(NODE_CODE)) != Some(ESCADRA_CODE)
    {
        return Err("invalid Shipworks fleet");
    }
    let (ally_entries, enemy_entries) = {
        let state = SESSION.try_lock().map_err(|_| "Shipworks roster is busy")?;
        (state.entries.clone(), state.enemy_entries.clone())
    };
    if !matches!(launch.roster, Roster::Strike) {
        let editor = editor_defender(allies).ok_or("missing editor defender property")?;
        let get_ship: unsafe extern "C" fn(usize) -> usize = std::mem::transmute(PROPERTY_SHIP);
        let current = get_ship(editor);
        if !diag::readable_span(current, size_of::<Creature>())
            || read::<u64>(current + NODE_CODE) != CREATURE_CODE
        {
            return Err("invalid editor defender");
        }
        let (ship, remove_original) = match launch.replay_for(Slot::Editor) {
            Some(Replay::Destroyed) => (None, true),
            Some(Replay::Saved(snapshot)) => {
                match design::append(allies, snapshot.path(), SIDE_ALLY) {
                    Some(ship) => {
                        log_ship_health("preserved editor ship", c"editor ship", ship);
                        (Some(ship), true)
                    }
                    None => {
                        log::warn!(
                            "hf_ally: damaged editor ship could not be loaded; using editor design"
                        );
                        (Some(current), false)
                    }
                }
            }
            None => (Some(current), false),
        };
        if remove_original {
            // SAFETY: FIGHT's original Property is the one native list child
            // validated above. Like Creature_SpawnFromTemplate (0x1402672c0),
            // invoke its native kill slot, not a Rust destructor or list edit.
            let vtable = read::<usize>(editor);
            let remove: unsafe extern "C" fn(usize) =
                std::mem::transmute(read::<usize>(vtable + 0x48));
            remove(editor);
        }
        if let Some(ship) = ship {
            launch.staged.push((ship, Slot::Editor));
        }
        append_defenders(launch, allies, &ally_entries, SIDE_ALLY, Slot::Ally);
    }
    if matches!(launch.roster, Roster::Strike | Roster::Mixed) {
        append_defenders(launch, enemies, &enemy_entries, SIDE_ENEMY, Slot::Enemy);
        if matches!(launch.roster, Roster::Strike) && launch.staged.is_empty() {
            return Err("no surviving enemy defenders; turn Preserve Damage off to reset");
        }
    }
    if launch.staged.is_empty() {
        return Err("no surviving defenders; turn Preserve Damage off to reset");
    }
    // SAFETY: both Links are initialized, live FIGHT vector elements. Native
    // reassignment/copy keeps their registrations and owns any vector growth.
    match launch.roster {
        Roster::Strike => {
            let set: SetLinkFn = std::mem::transmute(SET_LINK);
            set(ally_link, enemies);
        }
        Roster::Mixed
            if launch
                .staged
                .iter()
                .any(|(_, key)| matches!(key, Slot::Enemy(_))) =>
        {
            let push: unsafe extern "C" fn(usize, usize) = std::mem::transmute(PUSH_LINK);
            push(FRIENDLY_LIST, enemy_link);
        }
        Roster::Defense | Roster::AlliedDefense | Roster::Mixed => {}
    }
    Ok(())
}

unsafe fn editor_defender(fleet: usize) -> Option<usize> {
    let children = fleet + offset_of!(Node, children);
    let count = diag::rd::<usize>(children + offset_of!(SentinelList<usize>, length))?;
    let head = diag::rd::<usize>(children)?;
    let first = diag::rd::<usize>(head)?;
    if count != 1 || first == head || diag::rd::<usize>(first)? != head {
        return None;
    }
    diag::rd::<usize>(first + offset_of!(SentinelListNode<usize>, value))
        .filter(|&property| diag::readable_span(property, size_of::<Node>()))
        .filter(|&property| read::<usize>(property + design::MASTER_TARGET) == fleet)
}

unsafe fn append_defenders(
    launch: &mut Launch,
    fleet: usize,
    entries: &[super::Entry],
    side: f32,
    slot: impl Fn(usize) -> Slot,
) {
    for (index, entry) in entries.iter().enumerate() {
        let key = slot(index);
        let (ship, source) = match launch.replay_for(key) {
            Some(Replay::Destroyed) => continue,
            Some(Replay::Saved(snapshot)) => {
                let ship = design::append(fleet, snapshot.path(), side).or_else(|| {
                    log::warn!(
                        "hf_ally: damaged snapshot for {} could not be loaded; using design",
                        entry.name.to_string_lossy()
                    );
                    design::append(fleet, &entry.path, side)
                });
                (ship, "preserved snapshot")
            }
            None => (design::append(fleet, &entry.path, side), "design"),
        };
        if let Some(ship) = ship {
            log_ship_health(source, &entry.name, ship);
            launch.staged.push((ship, key));
        } else {
            log::warn!(
                "hf_ally: could not stage missile defender {}",
                entry.name.to_string_lossy()
            );
        }
    }
}

/// Diagnostics for Preserve Damage: log the live part damage on a staged ship.
unsafe fn log_ship_health(source: &str, name: &CStr, ship: usize) {
    if let Some((parts, damaged, health, health_max)) = design::health_summary(ship) {
        log::info!(
            "hf_ally: staged {source} {}: {damaged}/{parts} parts below max health, hull {health:.0}/{health_max:.0}",
            name.to_string_lossy()
        );
    }
}

unsafe fn single_link(vector: usize) -> Option<usize> {
    let (begin, count) = vector_span(vector, LINK_NODE_STRIDE, 1)?;
    (count == 1).then_some(begin)
}

unsafe extern "C" fn spawn_defenders() {
    let original: unsafe extern "C" fn() = std::mem::transmute(SPAWN_DEFENDERS);
    if !active() {
        original();
        log_campaign_roster("spawn");
        return;
    }
    let _hook = diag::scope("Shipworks missile scene spawn");
    // The native spawner unconditionally dereferences B[0]. Only enter it
    // after a native Missile has been constructed and registered there.
    let launch = BATTLE.try_lock().ok().and_then(|mut battle| battle.take());
    if let Some(launch) = launch.filter(|launch| prepare_missile(launch)) {
        write(TEST_FIGHT_FLAG, 1u8);
        original();
        if let Some(roster) = combat_roster() {
            damage::begin(&roster, &launch.staged, launch.preserve, launch.replay);
        }
    } else {
        log::warn!(
            "hf_ally: Shipworks Missile Defense missing its staged missile; returning to editor"
        );
        let load: LoadLevel = std::mem::transmute(LOAD_LEVEL);
        load(EDITOR_LEVEL.as_ptr(), 0, 0);
    }
}

unsafe fn combat_roster() -> Option<Vec<usize>> {
    let (begin, count) = vector_span(ROSTER_BEGIN, LINK_NODE_STRIDE, CUSTOM_DEFENDER_LIMIT)?;
    Some(
        (0..count)
            .map(|i| read::<usize>(begin + i * LINK_NODE_STRIDE + LINK_TARGET))
            .collect(),
    )
}

/// Campaign diagnostics: hull and part damage of the live missile-defense roster.
unsafe fn log_campaign_roster(phase: &str) {
    let Some(roster) = combat_roster() else {
        log::info!("hf_ally: campaign roster {phase}: unavailable");
        return;
    };
    log::info!("hf_ally: campaign roster {phase}: {} ships", roster.len());
    for (index, ship) in roster.into_iter().enumerate() {
        if let Some((parts, damaged, hull, hull_max)) = design::health_summary(ship) {
            log::info!(
                "hf_ally: campaign roster {phase} ship {index}: {damaged}/{parts} parts below max, hull {hull:.0}/{hull_max:.0}"
            );
        }
    }
}

unsafe extern "C" fn finish_defenders() {
    if active() {
        let _hook = diag::scope("Shipworks missile damage capture");
        if let Some(roster) = combat_roster() {
            damage::finish(&roster);
        } else {
            damage::invalidate_replay();
        }
    } else {
        log_campaign_roster("teardown");
    }
    // SAFETY: replay the original SCR_MISSILE teardown call once. It clears
    // the combat roster and moves surviving bodies out of the battle volume.
    let original: unsafe extern "C" fn() = std::mem::transmute(TEARDOWN);
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
        .iter()
        .any(|&ship| read::<f32>(ship + OFF_SIDE) * incoming < 0.0)
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

unsafe extern "C" fn set_airborne_gear(ship: usize, command: i32) {
    let broadcast: unsafe extern "C" fn(usize, i32) = std::mem::transmute(BROADCAST_GEAR_COMMAND);
    // SAFETY: this call site supplies the newly spawned Creature and the
    // original retract command. Only the airborne branch reaches it.
    broadcast(ship, command);
    if active()
        && diag::readable_span(ship, size_of::<Creature>())
        && read::<u64>(ship + NODE_CODE) == CREATURE_CODE
    {
        // As in BattleSpawner_SpawnShips (0x1402d21f0), snap the leg
        // joints and update their state before the physics tick. Changing
        // the command/state alone would still leave the legs extended.
        let snap: unsafe extern "C" fn(usize, u8) = std::mem::transmute(SNAP_FOLDED_GEAR);
        snap(ship, 0);
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
/// handler. In the Shipworks sandbox, run that handler and report the capped
/// tactical power so the trampoline can continue into the ordinary blast.
unsafe extern "C" fn nuclear_effect(body: usize) -> u32 {
    if !active() {
        log_campaign_nuke(body, "detonation");
        log_campaign_roster("detonation");
        // SAFETY: the native branch must run exactly once for every nuke body.
        let effects: unsafe extern "C" fn(usize) = std::mem::transmute(NUCLEAR_EFFECT);
        effects(body);
        return 0;
    }
    // Logic 10 reads the warhead stat at 0x140038410, then overwrites Body's
    // explosive power with 100 at 0x140038426 when switching to kind 4. That
    // value is for the stock nuclear path, not the energy of our tactical pass.
    // The map missile still has the catalog yield assigned by prepare_missile.
    // SAFETY: RCX is the live kind-4 Body at Body_Explode's original call. Only
    // its current Link to the matching, scene-owned Missile grants access to
    // that object's yield. Read it before the effect handler uses the Link.
    let body_power =
        diag::rd::<f32>(body.wrapping_add(offset_of!(Body, explosive_power))).unwrap_or(0.0);
    let map_yield =
        diag::rd::<usize>(body.wrapping_add(offset_of!(Body, projectile_link_2) + LINK_TARGET))
            .filter(|&missile| missile != 0 && missile == OWN_MISSILE.load(Ordering::Relaxed))
            .filter(|&missile| {
                diag::rd::<u64>(missile.wrapping_add(NODE_CODE)) == Some(MISSILE_CODE)
            })
            .and_then(|missile| diag::rd::<f32>(missile + MAP_MISSILE_EXPLOSIVE))
            .filter(|&yield_power| yield_power.is_finite() && yield_power > NUCLEAR_YIELD);

    let yield_power = map_yield.unwrap_or(body_power);
    let tactical_power = if yield_power.is_finite() && yield_power > 0.0 {
        yield_power
    } else {
        0.0
    };
    if let Some(map_yield) = map_yield {
        log::info!(
            "hf_ally: Shipworks nuclear blast: projectile power {body_power}, warhead yield {map_yield}, tactical power {tactical_power}"
        );
        log_blast_geometry(body, tactical_power);
    }
    // SAFETY: the native branch must run exactly once for every nuke body.
    // Keep the native projectile fields intact for the fuze, effect and map
    // handling; the sandbox damage is applied after the visual handler.
    let effects: unsafe extern "C" fn(usize) = std::mem::transmute(NUCLEAR_EFFECT);
    effects(body);
    apply_sandbox_nuclear_damage(body, tactical_power);
    // Returning zero keeps the vanilla kind-4 epilogue: no native area blast,
    // because the sandbox uses the campaign-matched part damage above.
    0
}

/// Damage the targeted fleet with the campaign strategic intensity.
///
/// The campaign `Explosion::Update` applies `min(cap, E / (1e6 * d^3))` to the
/// ships of the fleet the map missile reached. The tactical 400-unit proximity
/// fuse is **not** that map distance: at impact the map missile is at the
/// fleet, so the distance is the campaign floor (1.0 AI / 2.0 player) for every
/// ship in the targeted fleet, not each ship's tactical spacing. The exact
/// per-part bounding-box spread inside `0x1400e6aa0` is not decoded, so the
/// damage is spread as an evenly spaced sample of each ship's parts. A close
/// hit stays at the measured 2.5k-3.5k hull; a nuke that never reaches the
/// fleet would have no map impact and is not represented here.
unsafe fn apply_sandbox_nuclear_damage(body: usize, yield_power: f32) {
    if !yield_power.is_finite() || yield_power <= 0.0 {
        return;
    }
    let Some((blast_x, blast_y)) = body_world_position(body) else {
        return;
    };
    let Some(roster) = combat_roster() else {
        return;
    };
    // The missile's target link identifies the fleet the map missile reached.
    // If it is unavailable, keep the old behaviour and damage the whole roster.
    let target =
        diag::rd::<usize>(body.wrapping_add(offset_of!(Body, projectile_link_1) + LINK_TARGET))
            .unwrap_or(0);
    let target_side = if target != 0 && diag::readable_span(target, size_of::<Creature>()) {
        read::<f32>(target + OFF_SIDE)
    } else {
        0.0
    };
    for (index, ship) in roster.into_iter().enumerate() {
        let ship_side = read::<f32>(ship + OFF_SIDE);
        if target_side != 0.0 && ship_side * target_side <= 0.0 {
            continue;
        }
        let Some(_bridge) = diag::rd::<usize>(ship.wrapping_add(design::MASTER_TARGET))
            .filter(|&bridge| diag::readable_span(bridge, size_of::<Body>()))
            .filter(|&bridge| read::<usize>(bridge + NODE_OWNER) == ship)
        else {
            continue;
        };
        let player_side = ship_side > 0.0;
        let distance = if player_side {
            CAMPAIGN_NUKE_DISTANCE_FLOOR_PLAYER
        } else {
            CAMPAIGN_NUKE_DISTANCE_FLOOR_AI
        };
        let cap = if player_side {
            CAMPAIGN_NUKE_PLAYER_CAP
        } else {
            CAMPAIGN_NUKE_AI_CAP
        };
        let per_part =
            (yield_power / CAMPAIGN_NUKE_FALLOFF_DIVISOR / (distance * distance * distance))
                .min(cap);
        if !per_part.is_finite() || per_part <= 0.0 {
            continue;
        }
        log::info!(
            "hf_ally: sandbox nuke ship {index}: side {ship_side:+.0}, distance {distance:.1}, cap {cap:.0}, per-part {per_part:.4}"
        );
        let (hits, destroyed, lost) = damage_ship_parts(ship, (blast_x, blast_y), per_part);
        log::info!(
            "hf_ally: sandbox nuke ship {index}: {hits} parts hit, {destroyed} destroyed, {lost:.0} hull"
        );
    }
}

/// Spread one ship's campaign-intensity budget across the hull.
///
/// The campaign `0x1400e6aa0` distributes the intensity over the ship's part
/// bounding boxes; that exact spread is undecoded. A nearest-first pass
/// over-destroys because the ship's core is mostly 10-40 HP modules. Instead,
/// take an evenly spaced sample through the distance-sorted parts and apply at
/// most the campaign per-part damage to each. The master/root part is skipped:
/// the campaign `0x1400e6aa0` never kills it, and killing it removes the whole
/// ship even when the hull loss is small. On `Moral.seria` (379 parts) a close
/// hit gives ~24 destroyed parts and ~2,190 hull, matching the measured
/// campaign 11-24 / 1.8k-2.9k.
unsafe fn damage_ship_parts(ship: usize, center: (f32, f32), per_part: f32) -> (usize, usize, f32) {
    let Some(parts) = design::parts(ship) else {
        return (0, 0, 0.0);
    };
    let bridge = diag::rd::<usize>(ship.wrapping_add(design::MASTER_TARGET)).unwrap_or(0);
    let mut candidates: Vec<(usize, f32, f32)> = Vec::new();
    for part in parts {
        if part == bridge {
            continue;
        }
        let health = diag::rd::<f32>(part.wrapping_add(offset_of!(Body, health))).unwrap_or(0.0);
        if !health.is_finite() || health <= 0.0 {
            continue;
        }
        let Some((part_x, part_y)) = body_world_position(part) else {
            continue;
        };
        let dx = part_x - center.0;
        let dy = part_y - center.1;
        candidates.push((part, (dx * dx + dy * dy).sqrt(), health));
    }
    candidates.sort_by(|left, right| left.1.total_cmp(&right.1));
    if candidates.is_empty() {
        return (0, 0, 0.0);
    }
    let size_scale = (candidates.len() as f32 / SANDBOX_NUKE_REFERENCE_PARTS).clamp(0.75, 1.5);
    let count = ((SANDBOX_NUKE_DAMAGE_PER_MORAL / CAMPAIGN_NUKE_REFERENCE_DAMAGE) * size_scale)
        .round()
        .max(1.0) as usize;
    let stride = candidates.len() as f32 / count as f32;
    let mut budget = per_part * count as f32;
    let mut hits = 0usize;
    let mut destroyed = 0usize;
    let mut lost = 0.0f32;
    for index in 0..count {
        if budget <= 0.0 {
            break;
        }
        let candidate = ((index as f32 * stride) as usize).min(candidates.len() - 1);
        let (part, _, health) = candidates[candidate];
        let damage = per_part.min(health).min(budget);
        let spent = damage_part(part, damage);
        if spent > 0.0 {
            hits += 1;
            lost += spent;
            if health - spent <= 0.0 {
                destroyed += 1;
            }
        }
        budget -= spent;
    }
    (hits, destroyed, lost)
}

/// Call the native Body damage slot (vtable +0x210) and return the HP actually lost.
unsafe fn damage_part(part: usize, damage: f32) -> f32 {
    if damage <= 0.0 || !diag::readable_span(part, size_of::<Body>()) {
        return 0.0;
    }
    let health = diag::rd::<f32>(part.wrapping_add(offset_of!(Body, health))).unwrap_or(0.0);
    if !health.is_finite() || health <= 0.0 {
        return 0.0;
    }
    let state = diag::rd::<i32>(part.wrapping_add(offset_of!(Node, state))).unwrap_or(0);
    if state <= 1 {
        return 0.0;
    }
    let vtable = diag::rd::<usize>(part).unwrap_or(0);
    if vtable == 0 {
        return 0.0;
    }
    // SAFETY: a live Body's vtable carries the damage slot with the native
    // (damage, directHit, flag, hullContact, defer) contract. The caller
    // supplies roster-owned parts on the game thread.
    let apply: unsafe extern "C" fn(usize, f32, u8, u8, u8, u8) -> u8 =
        std::mem::transmute(read::<usize>(vtable + 0x210));
    apply(part, damage, 0, 1, 0, 0);
    let after = diag::rd::<f32>(part.wrapping_add(offset_of!(Body, health))).unwrap_or(0.0);
    (health - after).max(0.0)
}

/// Diagnostics: report where the tactical nuke detonated relative to each live defender.
/// Runs once per nuclear detonation while the combat roster is still populated.
unsafe fn log_blast_geometry(body: usize, tactical_power: f32) {
    let Some((blast_x, blast_y)) = body_world_position(body) else {
        return;
    };
    let radius = tactical_power.max(0.0).sqrt();
    let Some(roster) = combat_roster() else {
        return;
    };
    for (index, ship) in roster.into_iter().enumerate() {
        let Some(bridge) = diag::rd::<usize>(ship.wrapping_add(design::MASTER_TARGET))
            .filter(|&bridge| diag::readable_span(bridge, size_of::<Body>()))
            .filter(|&bridge| read::<usize>(bridge + NODE_OWNER) == ship)
        else {
            continue;
        };
        let Some((ship_x, ship_y)) = body_world_position(bridge) else {
            continue;
        };
        let dx = ship_x - blast_x;
        let dy = ship_y - blast_y;
        let distance = (dx * dx + dy * dy).sqrt();
        log::info!(
            "hf_ally: nuclear blast at ({blast_x:.1},{blast_y:.1}), {distance:.1} units from defender {index}, radius {radius:.1}"
        );
    }
}

/// Diagnostics: world position from the live Body's native getter (vtable +0xe8).
unsafe fn body_world_position(body: usize) -> Option<(f32, f32)> {
    if body == 0 || !diag::readable_span(body, size_of::<Body>()) {
        return None;
    }
    let vtable = diag::rd::<usize>(body)?;
    if vtable == 0 {
        return None;
    }
    // SAFETY: a live Body's vtable carries the world-position getter. The caller
    // supplies roster or exploding bodies on the game thread inside the scene.
    let get: unsafe extern "C" fn(usize, *mut f32) =
        std::mem::transmute(read::<usize>(vtable + 0xe8));
    let mut point = [0.0f32; 2];
    get(body, point.as_mut_ptr());
    (point[0].is_finite() && point[1].is_finite()).then_some((point[0], point[1]))
}

/// The strategic impact handler mutates the campaign Profile and the staged
/// escadra. A Shipworks test must not do that; its battle scene keeps its own
/// visual and tactical blast. Replay the native call outside our scene.
unsafe extern "C" fn nuclear_map_impact(target: usize, scale: f32, angle: f32) {
    if active() {
        return;
    }
    let impact: unsafe extern "C" fn(usize, f32, f32) = std::mem::transmute(NUCLEAR_MAP_IMPACT);
    let get_profile: unsafe extern "C" fn() -> usize = std::mem::transmute(GET_PROFILE);
    let profile = get_profile();
    let escalation_before = profile_escalation(profile);
    let explosions_before = profile_explosions_count(profile);
    let target_yield = diag::rd::<f32>(target.wrapping_add(MAP_MISSILE_EXPLOSIVE));
    let target_position = body_world_position(target);
    let target_raw_x = diag::rd::<f32>(target.wrapping_add(MARK_POSITION_X));
    let target_raw_y = diag::rd::<f32>(target.wrapping_add(MARK_POSITION_Y));
    let target_altitude = diag::rd::<f32>(target.wrapping_add(MARK_ALTITUDE));
    log::info!(
        "hf_ally: campaign nuclear impact: target {target:#x}, yield {target_yield:?}, getter {target_position:?}, map position ({target_raw_x:?},{target_raw_y:?}) altitude {target_altitude:?}, scale {scale}, angle {angle}, escalation {escalation_before:?}"
    );
    impact(target, scale, angle);
    let escalation_after = profile_escalation(profile);
    log::info!("hf_ally: campaign nuclear impact done: escalation {escalation_after:?}");
    log_last_explosion_mark(profile, explosions_before);
}

/// Campaign diagnostics: the strategic missile's kind-4 detonation on the game thread.
unsafe fn log_campaign_nuke(body: usize, phase: &str) {
    if body == 0 {
        return;
    }
    let body_power =
        diag::rd::<f32>(body.wrapping_add(offset_of!(Body, explosive_power))).unwrap_or(0.0);
    let map_missile = diag::rd::<usize>(body.wrapping_add(MAP_MISSILE_LINK)).unwrap_or(0);
    let map_yield = if map_missile != 0 && diag::readable_span(map_missile, MISSILE_SIZE) {
        diag::rd::<f32>(map_missile + MAP_MISSILE_EXPLOSIVE)
    } else {
        None
    };
    let position = body_world_position(body);
    let hit_body = diag::rd::<usize>(body.wrapping_add(0x3b8)).unwrap_or(0);
    let hit_owner = if hit_body == 0 {
        None
    } else {
        diag::rd::<usize>(hit_body.wrapping_add(NODE_OWNER))
    };
    log::info!(
        "hf_ally: campaign nuke {phase}: body {body:#x}, projectile power {body_power}, map missile {map_missile:#x}, map yield {map_yield:?}, position {position:?}, hit body {hit_body:#x}, hit owner {hit_owner:?}"
    );
}

unsafe fn profile_escalation(profile: usize) -> Option<i32> {
    if profile == 0 {
        return None;
    }
    diag::rd::<i32>(profile + PROFILE_ESCALATION)
}

unsafe fn profile_explosions_count(profile: usize) -> Option<usize> {
    if profile == 0 {
        return None;
    }
    let begin = diag::rd::<usize>(profile + PROFILE_EXPLOSIONS)?;
    let end = diag::rd::<usize>(profile + PROFILE_EXPLOSIONS + size_of::<usize>())?;
    end.checked_sub(begin)
        .map(|bytes| bytes / size_of::<usize>())
}

/// Log the Explosion mark added by one strategic impact, without holding any
/// pointer across a possible CVec reallocation.
unsafe fn log_last_explosion_mark(profile: usize, before: Option<usize>) {
    let Some(before) = before else {
        return;
    };
    let Some(after) = profile_explosions_count(profile) else {
        return;
    };
    if after <= before {
        return;
    }
    let Some(end) = diag::rd::<usize>(profile + PROFILE_EXPLOSIONS + size_of::<usize>()) else {
        return;
    };
    if end < size_of::<usize>() {
        return;
    }
    let Some(mark) = diag::rd::<usize>(end - size_of::<usize>()) else {
        return;
    };
    if mark == 0 || !diag::readable_span(mark, EXPLOSION_SHIP_DAMAGED + 1) {
        return;
    }
    let power = diag::rd::<f32>(mark + EXPLOSION_POWER).unwrap_or(0.0);
    let radius = diag::rd::<f32>(mark + EXPLOSION_RADIUS).unwrap_or(0.0);
    let yield_power = diag::rd::<f32>(mark + EXPLOSION_YIELD).unwrap_or(0.0);
    let nuclear = diag::rd::<u8>(mark + EXPLOSION_IS_NUCLEAR).unwrap_or(0);
    let ship_damaged = diag::rd::<u8>(mark + EXPLOSION_SHIP_DAMAGED).unwrap_or(0);
    let mark_x = diag::rd::<f32>(mark + MARK_POSITION_X);
    let mark_y = diag::rd::<f32>(mark + MARK_POSITION_Y);
    let mark_altitude = diag::rd::<f32>(mark + MARK_ALTITUDE);
    log::info!(
        "hf_ally: campaign explosion mark {mark:#x} ({} added): power {power}, radius {radius}, yield {yield_power}, nuclear {nuclear}, ship damaged {ship_damaged}, position ({mark_x:?},{mark_y:?}), altitude {mark_altitude:?}",
        after - before
    );
}

unsafe extern "C" fn keep_missile_exit() -> bool {
    if !active() {
        return false;
    }
    log::info!("hf_ally: Shipworks Missile Defense routing nuclear encounter to normal teardown");
    true
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
    damage::discard_encounter();
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
