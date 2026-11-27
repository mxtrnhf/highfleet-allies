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
use patchy_host::Condition;

use crate::patchy::{PatchSession, ReturnType, Trampoline};

use crate::memory::vector_span;
use crate::{
    memory,
    native::{live_ship, read, SetLinkFn, OFF_AI_FLAG, OFF_SIDE, ROSTER_BEGIN, SET_LINK},
    native_layout::{
        Body, Creature, EscadraString, Node, PartStats, SentinelList, NODE_CODE, NODE_OWNER,
    },
    sound,
    time::now_ms,
};

use super::{
    controls::{
        render_overlays as draw_sprites, scrollbar_art, show_slider, vertical_slider, wheel_input,
        EditorControls, OverlaySprite, SliderKind, PICKER_ROW_STYLE, PICKER_SELECTED_STYLE,
    },
    damage::{self, Replay, Scenario::MissileDefense, Slot},
    design, hide_control, native_string, prune_enemy_escadra, show_control, strategic_blast, write,
    ControlGeometry, BUTTON_GAP, BUTTON_STYLE, CLICK_DEBOUNCE_MS, ENEMY_LIMIT, ENEMY_LIST,
    LINK_CLEAR_VECTOR, LINK_NODE_STRIDE, LINK_TARGET, ROSTER_LIMIT, ROW_STYLE, SESSION, SIDE_ALLY,
    SIDE_ENEMY, TEST_FIGHT_FLAG,
};

const TYPE_NAME: &CStr = c"SW_ACE_MSL";
const START_NAME: &CStr = c"SW_ACE_START";
pub(super) const LAUNCH_NAME: &CStr = c"SW_ACE_LAUNCH";
const PRESERVE_NAME: &CStr = c"SW_ACE_KEEP";
const VIEW_NAME: &CStr = c"SW_ACE_DMG";
const PREV_NAME: &CStr = c"SW_DMG_PREV";
const NEXT_NAME: &CStr = c"SW_DMG_NEXT";
const PAGE_NAME: &CStr = c"SW_DMG_PAGE";
const CHOICE_NAMES: [&CStr; 5] = [
    c"SW_MSL_ROW1",
    c"SW_MSL_ROW2",
    c"SW_MSL_ROW3",
    c"SW_MSL_ROW4",
    c"SW_MSL_ROW5",
];
const SCROLL_UP: &CStr = c"SW_MSL_UP";
const SCROLL_DOWN: &CStr = c"SW_MSL_DOWN";
const SCROLL_BAR: &CStr = c"SW_MSL_BAR";
const MISSILE_LEVEL: &CStr = c"Missile.seria";
const EDITOR_LEVEL: &CStr = c"Editor.seria";
const FRAME_WIDTH: usize = 0x1_43a1_58fc;
const FRAME_HEIGHT: usize = 0x1_43a1_3e5c;
const SCHEMATIC_HEIGHT: f32 = 220.0;
const PANEL_RIGHT_MARGIN: f32 = 36.0;
const PANEL_WIDTH_IN_TEST_BUTTONS: f32 = 2.4;
const PANEL_ROW_GAP: f32 = 8.0;
const SCROLL_GUTTER_FRACTION: f32 = 0.16;
static TYPE_GUI: AtomicUsize = AtomicUsize::new(0);
static START_GUI: AtomicUsize = AtomicUsize::new(0);
static LAUNCH_GUI: AtomicUsize = AtomicUsize::new(0);
static PRESERVE_GUI: AtomicUsize = AtomicUsize::new(0);
static VIEW_GUI: AtomicUsize = AtomicUsize::new(0);
static PREV_GUI: AtomicUsize = AtomicUsize::new(0);
static NEXT_GUI: AtomicUsize = AtomicUsize::new(0);
static PAGE_GUI: AtomicUsize = AtomicUsize::new(0);
static CHOICE_GUIS: [AtomicUsize; CHOICE_NAMES.len()] =
    [const { AtomicUsize::new(0) }; CHOICE_NAMES.len()];
static SCROLL_UP_GUI: AtomicUsize = AtomicUsize::new(0);
static SCROLL_DOWN_GUI: AtomicUsize = AtomicUsize::new(0);
static SCROLL_BAR_GUI: AtomicUsize = AtomicUsize::new(0);
// Selected row, scrollbar track, and thumb.
static PICKER_OVERLAYS: Mutex<[Option<OverlaySprite>; 3]> = Mutex::new([None; 3]);

const STATS_TABLE: usize = 0x1_43a3_3bf0;
const STATS_LIMIT: usize = 4096;
// Stats_GetIndexMap (0x140281fa0) uses the first opaque EscadraString as the OID.
const STATS_OID: usize = offset_of!(PartStats, unknown_0048h);
const STATS_HOMING_TYPE: usize = 0x164;
const STATS_EXPLOSIVE: usize = 0x168;
const STRATEGIC_CATEGORY: u32 = 0x0800_0000;
const NUCLEAR_YIELD: f32 = 100_000.0;
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
const MISSILE_FROM_RIGHT: usize = 0x1_403c_2b69;
// SCR_MISSILE's four landed offsets from the defender edge are
// 2000, 1400, 3200 and 200 (0x1402d4ad1..0x1402d4b21, sign flips each slot).
const VANILLA_GROUND_FURTHEST_INSET: f32 = 3_200.0;
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
// The native logic-0x1518 effect owns the nuclear sequence and its scene exit.
const NUCLEAR_TEARDOWN_SITE: usize = 0x1_4004_0ed8;
const NUCLEAR_RETURN_SITE: usize = 0x1_4004_0ee9;
const SHADER_MAP_TECHNIQUE: usize = 0x118;
const SHADER_UNWARPED_TECHNIQUE: usize = 0x120;

const _: () = {
    assert!(STATS_OID == 0x48);
    assert!(size_of::<EscadraString>() == 0x20);
    assert!(offset_of!(Body, logic_active) == 0x3c8);
    assert!(offset_of!(Body, explosive_power) == 0x2dc);
    assert!(offset_of!(Body, projectile_link_1) + LINK_TARGET == 0x498);
    assert!(offset_of!(Body, projectile_link_2) + LINK_TARGET == MAP_MISSILE_LINK);
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
    picker: Option<Picker>,
    start: DefenderStart,
    preserve: bool,
    page: Option<usize>,
    last_click: Option<(&'static CStr, u64)>,
}

struct Picker {
    first: usize,
    visible: usize,
    // Apply arrow clicks on the next frame so clicks on rows still match the drawn list.
    pending_scroll: isize,
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
            picker: None,
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
        self.picker = None;
        self.last_click = None;
        self.open = true;
        log::info!(
            "ACES: Shipworks Missile Defense found {} launchable strategic missile types",
            self.choices.len()
        );
    }

    pub(super) fn close(&mut self) {
        self.open = false;
        self.picker = None;
        self.page = None;
        set_picker_overlays([None; 3]);
        damage::preview(MissileDefense, None);
    }

    /// Draw against the validated Shipworks TEST template and current GUI list.
    pub(super) unsafe fn draw(
        &mut self,
        listed: &[usize],
        template: usize,
        [width, height]: [f32; 2],
        top: f32,
        toolbar_right: f32,
        dropdown_open: bool,
    ) {
        if !self.open || dropdown_open {
            hide(listed);
            return;
        }
        let frame_width = read::<i32>(FRAME_WIDTH) as f32;
        let frame_height = read::<i32>(FRAME_HEIGHT) as f32;
        let panel_width =
            (width * PANEL_WIDTH_IN_TEST_BUTTONS).min(frame_width - 2.0 * PANEL_RIGHT_MARGIN);
        if panel_width <= 0.0 || frame_height <= 0.0 {
            hide(listed);
            return;
        }
        let panel_right = frame_width * 0.5 - PANEL_RIGHT_MARGIN;
        let x = panel_right - panel_width * 0.5;
        // Start level with TEST unless the right-hand stack would cover the toolbar.
        let y = if panel_right - panel_width >= toolbar_right + BUTTON_GAP {
            top
        } else {
            top + height + PANEL_ROW_GAP
        };
        let row = |y| ControlGeometry {
            position: [x, y],
            dimensions: [panel_width, height],
        };
        let step = height + PANEL_ROW_GAP;
        let list_top = y + height * 0.5 + PANEL_ROW_GAP;
        let list_bottom = frame_height * 0.5 - PANEL_ROW_GAP;
        let rows = (((list_bottom - list_top) / height) as usize)
            .min(CHOICE_NAMES.len())
            .min(self.choices.len());
        if rows == 0 {
            self.picker = None;
        }
        let missile = self.choices.get(self.selected);
        show_control(
            TYPE_NAME.to_bytes_with_nul(),
            &TYPE_GUI,
            listed,
            template,
            if self.picker.is_some() {
                c"CLOSE MISSILE LIST"
            } else {
                missile.map_or(c"NO STRATEGIC MISSILES", |missile| &missile.label)
            },
            row(y),
            BUTTON_STYLE,
        );
        if let Some(picker) = &mut self.picker {
            // The list occupies the setup area only while open. Keep the other settings intact.
            hide_setup(listed);
            hide_control(VIEW_NAME.to_bytes_with_nul(), &VIEW_GUI, listed);
            hide_viewer(listed);
            picker.draw(
                EditorControls { listed, template },
                [frame_width, frame_height],
                ControlGeometry {
                    position: [x, list_top + height * rows as f32 * 0.5],
                    dimensions: [panel_width, height * rows as f32],
                },
                rows,
                &self.choices,
                self.selected,
            );
            return;
        }
        hide_picker(listed);
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
            hide_setup(listed);
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
        let (count, page) = damage::pages(MissileDefense, page_index);
        let header_y = y + 5.0 * step;
        let title = page.as_ref().map_or_else(
            || c"NO COMBAT RESULTS".into(),
            |page| page.title(page_index, count),
        );
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
        damage::preview(
            MissileDefense,
            page.map(|page| {
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
            }),
        );
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
        if id == TYPE_NAME && !self.choices.is_empty() {
            self.picker = if self.picker.is_some() {
                set_picker_overlays([None; 3]);
                None
            } else {
                Some(Picker {
                    first: self.selected,
                    visible: 0,
                    pending_scroll: 0,
                })
            };
            sound::button("Shipworks missile picker");
            return false;
        }
        if let Some(picker) = &mut self.picker {
            if picker.visible == 0 {
                return false;
            }
            if id == SCROLL_UP && picker.first > 0 {
                picker.pending_scroll = picker.pending_scroll.saturating_sub(1);
            } else if id == SCROLL_DOWN && picker.first + picker.visible < self.choices.len() {
                picker.pending_scroll = picker.pending_scroll.saturating_add(1);
            } else if let Some(row) = CHOICE_NAMES
                .iter()
                .take(picker.visible)
                .position(|&name| name == id)
            {
                self.selected = picker.first + row;
                self.picker = None;
                set_picker_overlays([None; 3]);
            } else {
                return false;
            }
            sound::button("Shipworks missile picker");
            return false;
        }
        if id == VIEW_NAME {
            self.page = if self.page.is_some() { None } else { Some(0) };
            sound::button("Shipworks damage viewer");
            return false;
        }
        if id == PREV_NAME || id == NEXT_NAME {
            if let Some(index) = self.page {
                let (count, _) = damage::pages(MissileDefense, index);
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
        if id == START_NAME {
            self.start = match self.start {
                DefenderStart::Airborne => DefenderStart::Ground,
                DefenderStart::Ground => DefenderStart::Airborne,
            };
            sound::button("Shipworks missile defender start");
            return false;
        }
        if id != LAUNCH_NAME {
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
            damage::replay(MissileDefense)
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
        log::info!("ACES: Shipworks preserve replay: {saved} saved, {destroyed} destroyed");
        self.page = None;
        damage::preview(MissileDefense, None);
        sound::button("Shipworks missile launch");
        true
    }
}

impl Picker {
    /// Uses the current complete game-thread GUI scan and the validated TEST template.
    unsafe fn draw(
        &mut self,
        controls: EditorControls<'_>,
        frame_size: [f32; 2],
        area: ControlGeometry,
        rows: usize,
        choices: &[Missile],
        selected: usize,
    ) {
        let EditorControls { listed, template } = controls;
        let [x, y] = area.position;
        let [width, height] = area.dimensions;
        let left = x - width * 0.5;
        let right = x + width * 0.5;
        let top = y - height * 0.5;
        let bottom = y + height * 0.5;
        let row_height = height / rows as f32;
        let limit = choices.len().saturating_sub(rows);
        let mut scroll = std::mem::take(&mut self.pending_scroll);
        if limit > 0 {
            if let Some(([mouse_x, mouse_y], delta)) = wheel_input(frame_size) {
                if (left..right).contains(&mouse_x) && (top..bottom).contains(&mouse_y) {
                    scroll = scroll.saturating_sub(delta as isize);
                }
            }
        }
        let first = self.first.saturating_add_signed(scroll).min(limit);
        let gutter = if limit > 0 {
            width * SCROLL_GUTTER_FRACTION
        } else {
            0.0
        };
        let mut overlays = [None; 3];
        let [selected_art, track_art, thumb_art] = &mut overlays;
        if limit > 0 {
            let track = vertical_slider(ControlGeometry {
                position: [right - gutter * 0.75, y],
                dimensions: [gutter * 0.5, height],
            });
            // SCROLL_BAR is left to the native dispatcher, just like Airstrike's sliders.
            let value = show_slider(
                SCROLL_BAR,
                &SCROLL_BAR_GUI,
                controls,
                track,
                first as f32 / limit as f32,
                SliderKind::Scroll,
                first != self.first || rows != self.visible,
            );
            self.first = (value * limit as f32).round() as usize;
            let [track, thumb] = scrollbar_art(track, frame_size, rows, limit, self.first);
            *track_art = Some(track);
            *thumb_art = Some(thumb);
        } else {
            hide_control(SCROLL_BAR.to_bytes_with_nul(), &SCROLL_BAR_GUI, listed);
            self.first = 0;
        }
        self.visible = rows;

        for (row, (name, gui)) in CHOICE_NAMES.into_iter().zip(&CHOICE_GUIS).enumerate() {
            let index = self.first + row;
            if let Some(choice) = choices.get(index).filter(|_| row < rows) {
                let row_area = ControlGeometry {
                    position: [x - gutter * 0.5, top + row_height * (row as f32 + 0.5)],
                    dimensions: [width - gutter, row_height],
                };
                let style = if index == selected {
                    *selected_art = Some(OverlaySprite::selection(row_area, frame_size));
                    PICKER_SELECTED_STYLE
                } else {
                    PICKER_ROW_STYLE
                };
                show_control(
                    name.to_bytes_with_nul(),
                    gui,
                    listed,
                    template,
                    &choice.label,
                    row_area,
                    style,
                );
            } else {
                hide_control(name.to_bytes_with_nul(), gui, listed);
            }
        }
        set_picker_overlays(overlays);
        let arrow_height = row_height.min(height * 0.5);
        for (name, gui, label, visible, arrow_y) in [
            (
                SCROLL_UP,
                &SCROLL_UP_GUI,
                c"^",
                self.first > 0,
                top + arrow_height * 0.5,
            ),
            (
                SCROLL_DOWN,
                &SCROLL_DOWN_GUI,
                c"v",
                self.first < limit,
                bottom - arrow_height * 0.5,
            ),
        ] {
            if visible {
                show_control(
                    name.to_bytes_with_nul(),
                    gui,
                    listed,
                    template,
                    label,
                    ControlGeometry {
                        position: [right - gutter * 0.25, arrow_y],
                        dimensions: [gutter * 0.5, arrow_height],
                    },
                    PICKER_ROW_STYLE,
                );
            } else {
                hide_control(name.to_bytes_with_nul(), gui, listed);
            }
        }
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
        SCROLL_UP,
        SCROLL_DOWN,
    ]
    .into_iter()
    .chain(CHOICE_NAMES)
    .find(|id| id.to_bytes() == name.as_bytes())
}

pub(super) unsafe fn hide(listed: &[usize]) {
    hide_control(TYPE_NAME.to_bytes_with_nul(), &TYPE_GUI, listed);
    hide_control(VIEW_NAME.to_bytes_with_nul(), &VIEW_GUI, listed);
    hide_setup(listed);
    hide_picker(listed);
    hide_viewer(listed);
}

unsafe fn hide_setup(listed: &[usize]) {
    for (name, gui) in [
        (START_NAME, &START_GUI),
        (LAUNCH_NAME, &LAUNCH_GUI),
        (PRESERVE_NAME, &PRESERVE_GUI),
    ] {
        hide_control(name.to_bytes_with_nul(), gui, listed);
    }
}

unsafe fn hide_picker(listed: &[usize]) {
    for (name, gui) in CHOICE_NAMES.into_iter().zip(&CHOICE_GUIS).chain([
        (SCROLL_UP, &SCROLL_UP_GUI),
        (SCROLL_DOWN, &SCROLL_DOWN_GUI),
        (SCROLL_BAR, &SCROLL_BAR_GUI),
    ]) {
        hide_control(name.to_bytes_with_nul(), gui, listed);
    }
    set_picker_overlays([None; 3]);
}

fn set_picker_overlays(art: [Option<OverlaySprite>; 3]) {
    if let Ok(mut frame) = PICKER_OVERLAYS.try_lock() {
        *frame = art;
    }
}

pub(super) unsafe fn render_overlays() {
    if let Some(art) = PICKER_OVERLAYS.try_lock().ok().map(|frame| *frame) {
        draw_sprites(art.into_iter().flatten());
    }
}

unsafe fn hide_viewer(listed: &[usize]) {
    hide_control(PAGE_NAME.to_bytes_with_nul(), &PAGE_GUI, listed);
    hide_control(PREV_NAME.to_bytes_with_nul(), &PREV_GUI, listed);
    hide_control(NEXT_NAME.to_bytes_with_nul(), &NEXT_GUI, listed);
    damage::preview(MissileDefense, None);
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
            log::warn!("ACES: Shipworks Missile Defense FIGHT did not accept the editor ship");
        }
    }
}

pub(super) fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Copy strategic definitions from the live stats table, excluding records with
/// no actual spawnable Body in ALL_PARTS (the library DefinePart uses).
unsafe fn catalog() -> Vec<Missile> {
    let mut choices: Vec<Missile> = Vec::new();
    let Some((table, count)) = vector_span(STATS_TABLE, size_of::<usize>(), STATS_LIMIT) else {
        return choices;
    };
    let library = read::<usize>(ALL_PARTS);
    let Some(vtable) = memory::rd::<usize>(library) else {
        return choices;
    };
    let Some(find) = memory::rd::<usize>(vtable.wrapping_add(NODE_FIND_OID)).filter(|&p| p != 0)
    else {
        return choices;
    };
    let find: unsafe extern "C" fn(usize, *const i8) -> usize = std::mem::transmute(find);
    for index in 0..count {
        let stats = read::<usize>(table + index * size_of::<usize>());
        if !memory::readable_span(stats, STATS_EXPLOSIVE + size_of::<f32>())
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
        if !memory::readable_span(part, size_of::<Body>()) || read::<u64>(part + NODE_CODE) != 0xf {
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

/// Install the discrete SCR_MISSILE call sites. Vanilla paths replay
/// their original calls/branches. The modloader verifies the executable image.
pub(super) unsafe fn install(session: &mut PatchSession) -> Result<(), String> {
    for (site, handler, result) in [
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
        (
            NUCLEAR_TEARDOWN_SITE,
            finish_defenders as *const (),
            ReturnType::None,
        ),
        (
            NUCLEAR_RETURN_SITE,
            exit_missile as *const (),
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

    // Extend the vanilla four-ship loop only in Shipworks. Adjust only the
    // additional ships' positions before the native terrain clamp.
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

    // Kind 4 ends after the nuke effects. Shipworks uses the strategic blast
    // kernel, not the ordinary tactical explosion.
    let mut nuke = Trampoline::new();
    nuke.preserved_call(nuclear_effect as *const (), &[], &[], ReturnType::None);
    nuke.bytes(&[0xb0, 0x01]); // Keep the handled result for Body_DieExplode.
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
    damage::install(session)
}

unsafe extern "C" fn is_active() -> bool {
    active()
}

type LoadLevel = unsafe extern "C" fn(*const i8, u8, u8);

/// Load the original FIGHT scene unless an approved Missile Defense request is pending.
///
/// # Safety
/// Called synchronously from FIGHT's level-load site, with its original arguments
/// and with the final native fleet lists still live on the game thread.
pub(super) unsafe fn load_fight(path: *const i8, resume: u8, flag: u8) {
    let request = REQUEST
        .try_lock()
        .ok()
        .and_then(|mut request| request.take());
    let load: LoadLevel = std::mem::transmute(LOAD_LEVEL);
    let Some(mut launch) = request else {
        load(path, resume, flag);
        return;
    };
    match stage_defenders(&mut launch) {
        Ok(()) => {
            log::info!(
                "ACES: Shipworks Missile Defense launch {:?} / {:?}: {}",
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
        Err(reason) => log::warn!("ACES: Shipworks Missile Defense could not launch: {reason}"),
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
    if memory::rd::<u64>(allies.wrapping_add(NODE_CODE)) != Some(ESCADRA_CODE)
        || memory::rd::<u64>(enemies.wrapping_add(NODE_CODE)) != Some(ESCADRA_CODE)
    {
        return Err("invalid Shipworks fleet");
    }
    let (ally_entries, enemy_entries) = {
        let state = SESSION.try_lock().map_err(|_| "Shipworks roster is busy")?;
        (state.allies.entries.clone(), state.enemies.entries.clone())
    };
    if !matches!(launch.roster, Roster::Strike) {
        if let Some(ship) = damage::stage_editor(allies, &launch.replay)? {
            launch.staged.push((ship, Slot::Editor));
        }
        damage::stage_entries(
            allies,
            SIDE_ALLY,
            &ally_entries,
            Slot::Ally,
            &launch.replay,
            &mut launch.staged,
        );
    }
    if matches!(launch.roster, Roster::Strike | Roster::Mixed) {
        damage::stage_entries(
            enemies,
            SIDE_ENEMY,
            &enemy_entries,
            Slot::Enemy,
            &launch.replay,
            &mut launch.staged,
        );
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
    // The native spawner unconditionally dereferences B[0]. Only enter it
    // after a native Missile has been constructed and registered there.
    let launch = BATTLE.try_lock().ok().and_then(|mut battle| battle.take());
    if let Some(launch) = launch.filter(|launch| prepare_missile(launch)) {
        write(TEST_FIGHT_FLAG, 1u8);
        original();
        if let Some(roster) = MissileDefense.combat_roster() {
            damage::begin(
                MissileDefense,
                &roster,
                &launch.staged,
                launch.preserve,
                launch.replay,
            );
        }
    } else {
        log::warn!(
            "ACES: Shipworks Missile Defense missing its staged missile; returning to editor"
        );
        let load: LoadLevel = std::mem::transmute(LOAD_LEVEL);
        load(EDITOR_LEVEL.as_ptr(), 0, 0);
    }
}

/// Campaign diagnostics: hull and part damage of the live missile-defense roster.
unsafe fn log_campaign_roster(phase: &str) {
    let Some(roster) = MissileDefense.combat_roster() else {
        log::info!("ACES: campaign roster {phase}: unavailable");
        return;
    };
    log::info!("ACES: campaign roster {phase}: {} ships", roster.len());
    for (index, ship) in roster.into_iter().enumerate() {
        if let Some((parts, damaged, hull, hull_max)) = crate::ship_health::health_summary(ship) {
            log::info!(
                "ACES: campaign roster {phase} ship {index}: {damaged}/{parts} parts below max, hull {hull:.0}/{hull_max:.0}"
            );
        }
    }
}

unsafe extern "C" fn finish_defenders() {
    if active() {
        if let Some(roster) = MissileDefense.combat_roster() {
            damage::finish(&roster);
        } else {
            damage::invalidate_replay(MissileDefense);
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
        if !memory::readable_span(fleet, size_of::<Node>())
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
        let mut cell = memory::rd::<usize>(head)?;
        for _ in 0..count {
            if cell == head {
                return None;
            }
            let property =
                memory::rd::<usize>(cell.checked_add(offset_of!(SentinelListNode<usize>, value))?)?;
            if !memory::readable_span(property, size_of::<Node>()) {
                return None;
            }
            // SAFETY: these native property nodes and linked templates belong
            // to the freshly staged Escadra, on the scene's game thread.
            let ship = get_ship(property);
            if memory::readable_span(ship, size_of::<Creature>())
                && read::<u64>(ship + NODE_CODE) == CREATURE_CODE
                && has_thrust(ship) != 0
            {
                ships.push(ship);
            }
            cell = memory::rd::<usize>(cell)?;
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
        && memory::readable_span(ship, size_of::<Creature>())
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
    if !active() || index < VANILLA_DEFENDER_LIMIT {
        return;
    }
    let right = read::<f32>(ARENA_RIGHT);
    let left = read::<f32>(ARENA_LEFT);
    let span = right - left;
    let bottom = read::<f32>(ARENA_BOTTOM);
    if !span.is_finite() || span <= 2.0 * GROUND_EDGE_MARGIN || !bottom.is_finite() {
        return;
    }
    let from_right = read::<u8>(MISSILE_FROM_RIGHT) != 0;
    let extra = index - VANILLA_DEFENDER_LIMIT;
    let grounded = GROUNDED_DEFENDERS.load(Ordering::Relaxed);
    let (inset_from_defender_edge, y) = if grounded != 0 {
        // Continue past the furthest campaign slot towards the missile edge,
        // leaving clearance at that edge. The native clamp lands the extra
        // ship on the terrain using its hull height.
        let extras = grounded.saturating_sub(VANILLA_DEFENDER_LIMIT as usize);
        let room = span - VANILLA_GROUND_FURTHEST_INSET - GROUND_EDGE_MARGIN;
        if extra as usize >= extras || room <= 0.0 {
            return;
        }
        let spacing = (room / extras as f32).min(GROUND_MAX_SPACING);
        (
            VANILLA_GROUND_FURTHEST_INSET + (extra + 1) as f32 * spacing,
            bottom,
        )
    } else {
        // Keep the additional airborne ships in the existing rows.
        let column = extra % EXTRA_DEFENDERS_PER_ROW;
        let row = extra / EXTRA_DEFENDERS_PER_ROW;
        (
            span * 0.12 + column as f32 * span * 0.07,
            EXTRA_DEFENDER_HEIGHT - row as f32 * EXTRA_ROW_HEIGHT,
        )
    };
    let x = if from_right {
        left + inset_from_defender_edge
    } else {
        right - inset_from_defender_edge
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
                    if !memory::readable_span(ship, size_of::<Creature>())
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
                    (memory::readable_span(ship, size_of::<Creature>())
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
            "ACES: Shipworks P missile {body:#x} found no radar/jammer emitter; flying past (side {side:+})"
        );
    } else {
        log::info!(
            "ACES: Shipworks missile {body:#x} armed against {target:#x} (side {side:+}, homing {homing_type}, emission {emission:.1})"
        );
    }
    log::logger().flush();
}

/// Run the stock kind-4 effect and resolve the owned strike without a map load.
unsafe extern "C" fn nuclear_effect(body: usize) {
    let sandbox = active();
    if !sandbox {
        log_campaign_nuke(body, "detonation");
        log_campaign_roster("detonation");
    }

    // Read the target and warhead from the live projectile before the native
    // effects can use or change the links. Retain only scalar data.
    let impact = if sandbox {
        sandbox_nuclear_impact(body)
    } else {
        None
    };
    // SAFETY: RCX at the original kind-4 call is the live Body on the game
    // thread. Replay the native effect exactly once without changing its fields.
    let effects: unsafe extern "C" fn(usize) = std::mem::transmute(NUCLEAR_EFFECT);
    effects(body);
    if let Some((center, yield_power, target_side)) = impact {
        apply_sandbox_nuclear_damage(center, yield_power, target_side);
    }
}

unsafe fn sandbox_nuclear_impact(body: usize) -> Option<((f32, f32), f32, f32)> {
    // Logic 10 reads the warhead stat at 0x140038410, then overwrites Body's
    // explosive power with 100 at 0x140038426 when switching to kind 4.
    // The current Link to our staged Missile gives the catalog warhead yield.
    let missile =
        memory::rd::<usize>(body.wrapping_add(offset_of!(Body, projectile_link_2) + LINK_TARGET))
            .filter(|&missile| missile != 0 && missile == OWN_MISSILE.load(Ordering::Relaxed))
            .filter(|&missile| memory::rd::<u64>(missile + NODE_CODE) == Some(MISSILE_CODE))?;
    let yield_power = memory::rd::<f32>(missile + MAP_MISSILE_EXPLOSIVE)
        .filter(|&yield_power| yield_power.is_finite() && yield_power > NUCLEAR_YIELD)?;
    let missile_side = memory::rd::<f32>(missile + MAP_SIDE)?;

    // An unlinked fly-past or a lost target has no struck fleet. Do not fall
    // back to damaging both sides. Only the tactical defender link (not the
    // projectile side alone) identifies an impact against our combat roster.
    let target =
        memory::rd::<usize>(body.wrapping_add(offset_of!(Body, projectile_link_1) + LINK_TARGET))?;
    let roster = MissileDefense.combat_roster()?;
    if !roster.contains(&target)
        || !memory::readable_span(target, size_of::<Creature>())
        || !live_ship(target)
    {
        log::info!("ACES: Shipworks nuclear detonation without a live target; no fleet damage");
        return None;
    }
    let target_side = read::<f32>(target + OFF_SIDE);
    if !target_side.is_finite() || !missile_side.is_finite() || target_side * missile_side >= 0.0 {
        return None;
    }
    // Match Nuke_Detonate's raw centre reads at 0x14002bdcd / 0x14002bddd.
    let x = memory::rd::<f32>(body.wrapping_add(offset_of!(Body, position_x)))?;
    let y = memory::rd::<f32>(body.wrapping_add(offset_of!(Body, position_y)))?;
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let center = (x, y);
    log::info!(
        "ACES: Shipworks nuclear blast: warhead yield {yield_power}, target side {target_side:+.0}"
    );
    log_blast_geometry(center, yield_power, &roster);
    Some((center, yield_power, target_side))
}

/// Apply the campaign's hull damage stage to the live sandbox fleet, without
/// invoking the campaign's map simulation, Profile, or recovery scene.
unsafe fn apply_sandbox_nuclear_damage(center: (f32, f32), yield_power: f32, target_side: f32) {
    let Some(roster) = MissileDefense.combat_roster() else {
        return;
    };
    for (index, ship) in roster.into_iter().enumerate() {
        if !memory::readable_span(ship, size_of::<Creature>()) || !live_ship(ship) {
            continue;
        }
        let ship_side = read::<f32>(ship + OFF_SIDE);
        if !ship_side.is_finite() || ship_side * target_side <= 0.0 {
            continue;
        }
        let Some(bridge) = memory::rd::<usize>(ship.wrapping_add(design::MASTER_TARGET))
            .filter(|&bridge| memory::readable_span(bridge, size_of::<Body>()))
            .filter(|&bridge| read::<usize>(bridge + NODE_OWNER) == ship)
        else {
            continue;
        };
        // SAFETY: validated live roster Creature and its owned master, on
        // the game thread after Nuke_Detonate recorded the blast distance.
        let Some(result) =
            strategic_blast::apply(ship, bridge, center, yield_power, ship_side > 0.0)
        else {
            log::warn!("ACES: skipped invalid campaign blast geometry for defender {index}");
            continue;
        };
        log::info!(
            "ACES: sandbox nuke ship {index}: side {ship_side:+.0}, campaign distance {:.3}, column intensity {:.4}, {} parts hit, {} destroyed, {:.0} hull",
            result.distance,
            result.intensity,
            result.hit,
            result.destroyed,
            result.hull_lost,
        );
    }
}

/// Diagnostics: report where the tactical nuke detonated relative to each live defender.
/// Runs once per nuclear detonation while the combat roster is still populated.
unsafe fn log_blast_geometry((blast_x, blast_y): (f32, f32), yield_power: f32, roster: &[usize]) {
    // The yield suggests an equivalent tactical radius, not the radius used
    // for Shipworks' fleet-level damage.
    let radius = yield_power.sqrt();
    for (index, &ship) in roster.iter().enumerate() {
        let Some(bridge) = memory::rd::<usize>(ship.wrapping_add(design::MASTER_TARGET))
            .filter(|&bridge| memory::readable_span(bridge, size_of::<Body>()))
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
            "ACES: nuclear blast at ({blast_x:.1},{blast_y:.1}), {distance:.1} units from defender {index}, equivalent tactical radius {radius:.1}"
        );
    }
}

/// Diagnostics: world position from the live Body's native getter (vtable +0xe8).
unsafe fn body_world_position(body: usize) -> Option<(f32, f32)> {
    if body == 0 || !memory::readable_span(body, size_of::<Body>()) {
        return None;
    }
    let vtable = memory::rd::<usize>(body)?;
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
    let target_yield = memory::rd::<f32>(target.wrapping_add(MAP_MISSILE_EXPLOSIVE));
    let target_position = body_world_position(target);
    let target_raw_x = memory::rd::<f32>(target.wrapping_add(MARK_POSITION_X));
    let target_raw_y = memory::rd::<f32>(target.wrapping_add(MARK_POSITION_Y));
    let target_altitude = memory::rd::<f32>(target.wrapping_add(MARK_ALTITUDE));
    log::info!(
        "ACES: campaign nuclear impact: target {target:#x}, yield {target_yield:?}, getter {target_position:?}, map position ({target_raw_x:?},{target_raw_y:?}) altitude {target_altitude:?}, scale {scale}, angle {angle}, escalation {escalation_before:?}"
    );
    impact(target, scale, angle);
    let escalation_after = profile_escalation(profile);
    log::info!("ACES: campaign nuclear impact done: escalation {escalation_after:?}");
    log_last_explosion_mark(profile, explosions_before);
}

/// Campaign diagnostics: the strategic missile's kind-4 detonation on the game thread.
unsafe fn log_campaign_nuke(body: usize, phase: &str) {
    if body == 0 {
        return;
    }
    let body_power =
        memory::rd::<f32>(body.wrapping_add(offset_of!(Body, explosive_power))).unwrap_or(0.0);
    let map_missile = memory::rd::<usize>(body.wrapping_add(MAP_MISSILE_LINK)).unwrap_or(0);
    let map_yield = if map_missile != 0 && memory::readable_span(map_missile, MISSILE_SIZE) {
        memory::rd::<f32>(map_missile + MAP_MISSILE_EXPLOSIVE)
    } else {
        None
    };
    let position = body_world_position(body);
    let hit_body = memory::rd::<usize>(body.wrapping_add(0x3b8)).unwrap_or(0);
    let hit_owner = if hit_body == 0 {
        None
    } else {
        memory::rd::<usize>(hit_body.wrapping_add(NODE_OWNER))
    };
    log::info!(
        "ACES: campaign nuke {phase}: body {body:#x}, projectile power {body_power}, map missile {map_missile:#x}, map yield {map_yield:?}, position {position:?}, hit body {hit_body:#x}, hit owner {hit_owner:?}"
    );
}

unsafe fn profile_escalation(profile: usize) -> Option<i32> {
    if profile == 0 {
        return None;
    }
    memory::rd::<i32>(profile + PROFILE_ESCALATION)
}

unsafe fn profile_explosions_count(profile: usize) -> Option<usize> {
    if profile == 0 {
        return None;
    }
    let begin = memory::rd::<usize>(profile + PROFILE_EXPLOSIONS)?;
    let end = memory::rd::<usize>(profile + PROFILE_EXPLOSIONS + size_of::<usize>())?;
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
    let Some(end) = memory::rd::<usize>(profile + PROFILE_EXPLOSIONS + size_of::<usize>()) else {
        return;
    };
    if end < size_of::<usize>() {
        return;
    }
    let Some(mark) = memory::rd::<usize>(end - size_of::<usize>()) else {
        return;
    };
    if mark == 0 || !memory::readable_span(mark, EXPLOSION_SHIP_DAMAGED + 1) {
        return;
    }
    let power = memory::rd::<f32>(mark + EXPLOSION_POWER).unwrap_or(0.0);
    let radius = memory::rd::<f32>(mark + EXPLOSION_RADIUS).unwrap_or(0.0);
    let yield_power = memory::rd::<f32>(mark + EXPLOSION_YIELD).unwrap_or(0.0);
    let nuclear = memory::rd::<u8>(mark + EXPLOSION_IS_NUCLEAR).unwrap_or(0);
    let ship_damaged = memory::rd::<u8>(mark + EXPLOSION_SHIP_DAMAGED).unwrap_or(0);
    let mark_x = memory::rd::<f32>(mark + MARK_POSITION_X);
    let mark_y = memory::rd::<f32>(mark + MARK_POSITION_Y);
    let mark_altitude = memory::rd::<f32>(mark + MARK_ALTITUDE);
    log::info!(
        "ACES: campaign explosion mark {mark:#x} ({} added): power {power}, radius {radius}, yield {yield_power}, nuclear {nuclear}, ship damaged {ship_damaged}, position ({mark_x:?},{mark_y:?}), altitude {mark_altitude:?}",
        after - before
    );
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
    memory::rd::<usize>(shader.wrapping_add(technique))
        .filter(|&pointer| pointer != 0)
        .or_else(|| memory::rd::<usize>(shader.wrapping_add(SHADER_MAP_TECHNIQUE)))
        .unwrap_or(0)
}

unsafe extern "C" fn exit_missile(path: *const i8, resume: u8, flag: u8) {
    let load: LoadLevel = std::mem::transmute(LOAD_LEVEL);
    load(
        if active() || super::airstrike::active() {
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
                && memory::rd::<u64>(missile.wrapping_add(NODE_CODE)) == Some(MISSILE_CODE)
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
    log::info!("ACES: Shipworks Missile Defense ended; cleared staging links");
}
