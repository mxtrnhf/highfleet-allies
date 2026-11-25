//! Standalone Shipworks Airstrike panel. The encounter adapter never uses the
//! player's inventory; this panel copies choices from the live aircraft tables.

mod encounter;

use std::{
    ffi::{CStr, CString},
    mem::{offset_of, size_of},
    sync::{atomic::AtomicUsize, Mutex},
};

use crate::{
    diag,
    native::read,
    native_layout::{Body, NODE_CODE},
    plane_loadouts, sound,
    time::now_ms,
};

use super::{
    controls::{
        render_overlays as draw_sprites, scrollbar_art, show_slider, vertical_slider, wheel_input,
        EditorControls, OverlaySprite, SliderKind, PICKER_ROW_STYLE, PICKER_SELECTED_STYLE,
    },
    damage::{self, Replay, Scenario::Airstrike, Slot},
    hide_control, show_control, ControlGeometry, BUTTON_GAP, BUTTON_STYLE, CLICK_DEBOUNCE_MS,
    ENEMY_ROW_STYLE, ROW_STYLE,
};

pub(super) use encounter::{
    active, cancel_request, cancel_unhandled_request, install, leave, load_fight, requested,
    DEFENDER_LIMIT, DEFENDER_ROSTER,
};

pub(super) const BUTTON: &CStr = c"SW_ACE_AIR";
pub(super) const LAUNCH: &CStr = c"SW_AIR_LAUNCH";
pub(super) const LEVEL: &CStr = c"Bombers.seria";
const ADD: &CStr = c"SW_AIR_ADD";
const RESET: &CStr = c"SW_AIR_RESET";
const START: &CStr = c"SW_AIR_START";
const STATUS: &CStr = c"SW_AIR_STATUS";
const PAGE: &CStr = c"SW_AIR_PAGE";
const PRESERVE: &CStr = c"SW_AIR_KEEP";
const VIEW: &CStr = c"SW_AIR_DMG";
const PREV: &CStr = c"SW_AIR_PREV";
const NEXT: &CStr = c"SW_AIR_NEXT";
const RESULT: &CStr = c"SW_AIR_RESULT";
const HEADER_CONTROLS: [&CStr; 11] = [
    ADD, RESET, START, LAUNCH, STATUS, PAGE, PRESERVE, VIEW, PREV, NEXT, RESULT,
];
static HEADER_GUIS: [AtomicUsize; HEADER_CONTROLS.len()] =
    [const { AtomicUsize::new(0) }; HEADER_CONTROLS.len()];

const ROWS_PER_PAGE: usize = 4;
const ENTRY_LIMIT: usize = 8;
const MAX_PLANES: u8 = 15;
const MAX_MULTIPLIER: u8 = 8;
const LOADOUT_LINES: usize = 3;
const CRAFT: usize = 0;
const REMOVE: usize = 1;
const FINE: usize = 2;
const MULTIPLIER: usize = 3;
const FIRST_LOADOUT: usize = 4;
const SCROLL_UP: usize = FIRST_LOADOUT + LOADOUT_LINES;
const SCROLL_DOWN: usize = SCROLL_UP + 1;
const SCROLL_BAR: usize = SCROLL_DOWN + 1;
const GROUP_CONTROLS: usize = SCROLL_BAR + 1;
const ROW_CONTROLS: [[&CStr; GROUP_CONTROLS]; ROWS_PER_PAGE] = [
    [
        c"SW_AIR_CRAFT1",
        c"SW_AIR_DEL1",
        c"SW_AIR_COUNT1",
        c"SW_AIR_MULT1",
        c"SW_AIR_L1A",
        c"SW_AIR_L1B",
        c"SW_AIR_L1C",
        c"SW_AIR_UP1",
        c"SW_AIR_DOWN1",
        c"SW_AIR_BAR1",
    ],
    [
        c"SW_AIR_CRAFT2",
        c"SW_AIR_DEL2",
        c"SW_AIR_COUNT2",
        c"SW_AIR_MULT2",
        c"SW_AIR_L2A",
        c"SW_AIR_L2B",
        c"SW_AIR_L2C",
        c"SW_AIR_UP2",
        c"SW_AIR_DOWN2",
        c"SW_AIR_BAR2",
    ],
    [
        c"SW_AIR_CRAFT3",
        c"SW_AIR_DEL3",
        c"SW_AIR_COUNT3",
        c"SW_AIR_MULT3",
        c"SW_AIR_L3A",
        c"SW_AIR_L3B",
        c"SW_AIR_L3C",
        c"SW_AIR_UP3",
        c"SW_AIR_DOWN3",
        c"SW_AIR_BAR3",
    ],
    [
        c"SW_AIR_CRAFT4",
        c"SW_AIR_DEL4",
        c"SW_AIR_COUNT4",
        c"SW_AIR_MULT4",
        c"SW_AIR_L4A",
        c"SW_AIR_L4B",
        c"SW_AIR_L4C",
        c"SW_AIR_UP4",
        c"SW_AIR_DOWN4",
        c"SW_AIR_BAR4",
    ],
];
static ROW_GUIS: [[AtomicUsize; GROUP_CONTROLS]; ROWS_PER_PAGE] =
    [const { [const { AtomicUsize::new(0) }; GROUP_CONTROLS] }; ROWS_PER_PAGE];

const FRAME_WIDTH: usize = 0x1_43a1_58fc;
const FRAME_HEIGHT: usize = 0x1_43a1_3e5c;
const PANEL_MARGIN: f32 = 36.0;
const ROW_GAP: f32 = 6.0;
const ALL_PARTS: usize = 0x1_43a1_39f0;

struct Craft {
    key: CString,
    label: CString,
    icon: &'static CStr,
    loadouts: Vec<Loadout>,
}

struct Loadout {
    key: CString,
    label: CString,
}

#[derive(Clone, Copy)]
struct Entry {
    craft: usize,
    loadout: usize,
    count: u8,
    multiplier: u8,
    first_loadout: usize,
    restore_scroll: bool,
}

struct Flight {
    craft: CString,
    loadout: CString,
    count: u8,
}

struct Launch {
    flights: Vec<Flight>,
    friendly: bool,
    ground: bool,
    preserve: bool,
    replay: Vec<(Slot, Replay)>,
    staged: Vec<(usize, Slot)>,
}

pub(super) struct Panel {
    open: bool,
    crafts: Vec<Craft>,
    entries: Vec<Entry>,
    page: usize,
    restore_counts: bool,
    ground: bool,
    preserve: bool,
    damage_page: Option<usize>,
    error: Option<&'static CStr>,
    last_click: Option<(&'static CStr, u64)>,
}

// Icon, selected loadout, scrollbar track, and thumb for each group.
static OVERLAYS: Mutex<[[Option<OverlaySprite>; 4]; ROWS_PER_PAGE]> =
    Mutex::new([[None; 4]; ROWS_PER_PAGE]);

impl Panel {
    pub(super) const fn new() -> Self {
        Self {
            open: false,
            crafts: Vec::new(),
            entries: Vec::new(),
            page: 0,
            restore_counts: true,
            ground: false,
            preserve: false,
            damage_page: None,
            error: None,
            last_click: None,
        }
    }

    /// Game-thread input after the modded craft and part libraries are initialized.
    pub(super) unsafe fn open(&mut self) {
        let crafts = catalog();
        self.entries.retain_mut(|entry| {
            let Some((old_craft, old_loadout)) = selection(&self.crafts, *entry) else {
                return false;
            };
            let Some((craft, loadout)) = find_selection(&crafts, &old_craft.key, &old_loadout.key)
            else {
                return false;
            };
            entry.craft = craft;
            entry.loadout = loadout;
            entry.first_loadout = loadout.saturating_sub(LOADOUT_LINES - 1);
            true
        });
        self.crafts = crafts;
        self.page = self
            .page
            .min(self.entries.len().saturating_sub(1) / ROWS_PER_PAGE);
        self.restore_counts = true;
        self.error = None;
        self.last_click = None;
        self.open = true;
        log::info!(
            "hf_ally: Shipworks Airstrike found {} craft types",
            self.crafts.len()
        );
    }

    pub(super) fn close(&mut self) {
        self.open = false;
        self.damage_page = None;
        self.error = None;
        clear_overlays();
        damage::preview(Airstrike, None);
    }

    /// Draw from a complete editor GUI scan, before the native screen render.
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
        let panel_width = (width * 3.8).min(frame_width - 2.0 * PANEL_MARGIN);
        if panel_width <= 0.0 || frame_height <= 0.0 {
            hide(listed);
            return;
        }
        let right = frame_width * 0.5 - PANEL_MARGIN;
        let left = right - panel_width;
        let top = if left < toolbar_right + BUTTON_GAP {
            top + height + ROW_GAP
        } else {
            top
        };
        let cell = |from: f32, to: f32, y| ControlGeometry {
            position: [left + panel_width * 0.5 * (from + to), y],
            dimensions: [panel_width * (to - from) - ROW_GAP, height],
        };
        let show = |index: usize, text: &CStr, area, style| {
            show_control(
                HEADER_CONTROLS[index].to_bytes_with_nul(),
                &HEADER_GUIS[index],
                listed,
                template,
                text,
                area,
                style,
            );
        };
        let step = height + ROW_GAP;
        let preserve_label = if self.preserve {
            c"[X] PRESERVE DAMAGE"
        } else {
            c"[ ] PRESERVE DAMAGE"
        };
        if let Some(index) = self.damage_page {
            show(6, preserve_label, cell(0.0, 0.65, top), BUTTON_STYLE);
            show(7, c"BACK", cell(0.65, 1.0, top), BUTTON_STYLE);
            for control in 0..6 {
                hide_control(
                    HEADER_CONTROLS[control].to_bytes_with_nul(),
                    &HEADER_GUIS[control],
                    listed,
                );
            }
            for (names, guis) in ROW_CONTROLS.iter().zip(&ROW_GUIS) {
                hide_row(listed, names, guis);
            }
            clear_overlays();
            let (count, page) = damage::pages(Airstrike, index);
            let title = page.as_ref().map_or_else(
                || c"NO COMBAT RESULTS".into(),
                |page| page.title(index, count),
            );
            let mut title_area = cell(0.0, 1.0, top + step);
            title_area.position[0] = left;
            show(10, &title, title_area, ROW_STYLE);
            let schematic_top = top + step + height * 0.5 + ROW_GAP;
            let schematic_height = 220.0f32
                .min((frame_height * 0.5 - schematic_top - height - 2.0 * ROW_GAP).max(0.0));
            damage::preview(
                Airstrike,
                page.filter(|_| schematic_height > 0.0).map(|page| {
                    let mut area = cell(0.0, 1.0, schematic_top + schematic_height * 0.5);
                    area.position[0] += frame_width * 0.5;
                    area.position[1] += frame_height * 0.5;
                    area.dimensions[1] = schematic_height;
                    (area, page)
                }),
            );
            let nav_y = schematic_top + schematic_height + height * 0.5 + ROW_GAP;
            if count > 1 {
                show(8, c"PREV", cell(0.0, 0.5, nav_y), BUTTON_STYLE);
                show(9, c"NEXT", cell(0.5, 1.0, nav_y), BUTTON_STYLE);
            } else {
                for control in 8..10 {
                    hide_control(
                        HEADER_CONTROLS[control].to_bytes_with_nul(),
                        &HEADER_GUIS[control],
                        listed,
                    );
                }
            }
            return;
        }
        for control in 8..11 {
            hide_control(
                HEADER_CONTROLS[control].to_bytes_with_nul(),
                &HEADER_GUIS[control],
                listed,
            );
        }
        damage::preview(Airstrike, None);
        show(0, c"ADD PLANE", cell(0.0, 0.55, top), BUTTON_STYLE);
        show(1, c"RESET", cell(0.55, 1.0, top), BUTTON_STYLE);
        let top = top + step;
        show(6, preserve_label, cell(0.0, 0.65, top), BUTTON_STYLE);
        show(7, c"DMG VIEW", cell(0.65, 1.0, top), BUTTON_STYLE);
        let frame_size = [frame_width, frame_height];
        let controls = EditorControls { listed, template };
        let wheel = wheel_input(frame_size);
        let first = self.page * ROWS_PER_PAGE;
        let mut y = top + height * 0.5 + ROW_GAP;
        // One compact left stack and three list rows side by side. Reserve the
        // footer before sizing the four rows so it stays reachable.
        let available = frame_height * 0.5 - y - step - height * 0.65;
        let group_height =
            (available / ROWS_PER_PAGE as f32 - ROW_GAP).clamp(height * 0.75, height * 1.4);
        let list_step = group_height / LOADOUT_LINES as f32;
        let mut overlays = [[None; 4]; ROWS_PER_PAGE];
        for (index, ((names, guis), art)) in ROW_CONTROLS
            .iter()
            .zip(&ROW_GUIS)
            .zip(&mut overlays)
            .enumerate()
        {
            let [icon, selected_art, track_art, thumb_art] = art;
            let Some(entry) = self.entries.get_mut(first + index) else {
                hide_row(listed, names, guis);
                continue;
            };
            let Some((craft, _)) = selection(&self.crafts, *entry) else {
                hide_row(listed, names, guis);
                continue;
            };
            let show = |control: usize, label: &CStr, area, style| {
                show_control(
                    names[control].to_bytes_with_nul(),
                    &guis[control],
                    listed,
                    template,
                    label,
                    area,
                    style,
                );
            };
            let group_cell = |from, to, offset: f32, ratio: f32| {
                let mut area = cell(from, to, y + group_height * offset);
                area.dimensions[1] = group_height * ratio;
                area
            };
            show(
                CRAFT,
                &craft.label,
                group_cell(0.10, 0.39, 0.21, 0.42),
                BUTTON_STYLE,
            );
            show(
                REMOVE,
                c"X",
                group_cell(0.39, 0.47, 0.21, 0.42),
                BUTTON_STYLE,
            );
            for (control, value, kind, offset) in [
                (
                    FINE,
                    &mut entry.count,
                    SliderKind::Integer {
                        max: MAX_PLANES,
                        suffix: c" PLANES",
                    },
                    0.57,
                ),
                (
                    MULTIPLIER,
                    &mut entry.multiplier,
                    SliderKind::Integer {
                        max: MAX_MULTIPLIER,
                        suffix: c"x",
                    },
                    0.85,
                ),
            ] {
                *value = show_slider(
                    names[control],
                    &guis[control],
                    controls,
                    group_cell(0.0, 0.47, offset, 0.23),
                    f32::from(*value),
                    kind,
                    self.restore_counts,
                )
                .floor() as u8;
            }
            let list_left = left + panel_width * 0.47;
            let list_bottom = y + group_height;
            if let Some(([x, mouse_y], delta)) = wheel {
                if (list_left..right).contains(&x) && (y..list_bottom).contains(&mouse_y) {
                    entry.scroll_loadouts(craft, -(delta as isize));
                }
            }
            let scroll_limit = craft.loadouts.len().saturating_sub(LOADOUT_LINES);
            if scroll_limit != 0 {
                let track = vertical_slider(group_cell(0.88, 0.95, 0.5, 0.96));
                let limit = scroll_limit as f32;
                let value = show_slider(
                    names[SCROLL_BAR],
                    &guis[SCROLL_BAR],
                    controls,
                    track,
                    entry.first_loadout as f32 / limit,
                    SliderKind::Scroll,
                    self.restore_counts || entry.restore_scroll,
                );
                entry.restore_scroll = false;
                entry.first_loadout = (value * limit).round() as usize;
                let [track, thumb] = scrollbar_art(
                    track,
                    frame_size,
                    LOADOUT_LINES,
                    scroll_limit,
                    entry.first_loadout,
                );
                *track_art = Some(track);
                *thumb_art = Some(thumb);
            } else {
                hide_control(
                    names[SCROLL_BAR].to_bytes_with_nul(),
                    &guis[SCROLL_BAR],
                    listed,
                );
            }
            for line in 0..LOADOUT_LINES {
                let control = FIRST_LOADOUT + line;
                let loadout = entry.first_loadout + line;
                if let Some(option) = craft.loadouts.get(loadout) {
                    let list_right = if scroll_limit == 0 { 1.0 } else { 0.88 };
                    let mut area = cell(0.47, list_right, y + list_step * (line as f32 + 0.5));
                    area.dimensions[1] = list_step;
                    let style = if loadout == entry.loadout {
                        *selected_art = Some(OverlaySprite::selection(area, frame_size));
                        PICKER_SELECTED_STYLE
                    } else {
                        PICKER_ROW_STYLE
                    };
                    show(control, &option.label, area, style);
                } else {
                    hide_control(names[control].to_bytes_with_nul(), &guis[control], listed);
                }
            }
            for (control, label, visible, line) in [
                (SCROLL_UP, c"^", entry.first_loadout > 0, 0.5),
                (
                    SCROLL_DOWN,
                    c"v",
                    entry.first_loadout + LOADOUT_LINES < craft.loadouts.len(),
                    2.5,
                ),
            ] {
                if visible {
                    let mut area = cell(0.95, 1.0, y + list_step * line);
                    area.dimensions[1] = list_step;
                    show(control, label, area, PICKER_ROW_STYLE);
                } else {
                    hide_control(names[control].to_bytes_with_nul(), &guis[control], listed);
                }
            }
            let size = (group_height * 0.48).min(panel_width * 0.10);
            *icon = Some(OverlaySprite::icon(
                craft.icon,
                ControlGeometry {
                    position: [left + panel_width * 0.05, y + group_height * 0.21],
                    dimensions: [size, size],
                },
                frame_size,
            ));
            y = list_bottom + ROW_GAP;
        }
        if let Ok(mut frame) = OVERLAYS.try_lock() {
            *frame = overlays;
        }
        self.restore_counts = false;

        let y = y + height * 0.5;
        let pages = self.entries.len().div_ceil(ROWS_PER_PAGE);
        if pages > 1 {
            if let Ok(page) = CString::new(format!("PAGE {} / {}  >", self.page + 1, pages)) {
                show(5, &page, cell(0.0, 0.30, y), BUTTON_STYLE);
            }
        } else {
            hide_control(PAGE.to_bytes_with_nul(), &HEADER_GUIS[5], listed);
        }
        let start_left = if pages > 1 { 0.30 } else { 0.0 };
        let half = (1.0 + start_left) * 0.5;
        show(
            2,
            if self.ground { c"GROUND" } else { c"AIRBORNE" },
            cell(start_left, half, y),
            BUTTON_STYLE,
        );
        show(3, c"LAUNCH", cell(half, 1.0, y), BUTTON_STYLE);
        if let Some(error) = self
            .error
            .or(self.crafts.is_empty().then_some(c"NO AIRCRAFT AVAILABLE"))
        {
            let mut area = cell(0.0, 1.0, y + step);
            area.position[0] = left;
            show(4, error, area, ENEMY_ROW_STYLE);
        } else {
            hide_control(STATUS.to_bytes_with_nul(), &HEADER_GUIS[4], listed);
        }
    }

    /// Return true only for an accepted LAUNCH, to alias the native FIGHT click.
    pub(super) unsafe fn click(&mut self, name: &str, allies: usize, enemies: usize) -> bool {
        if !self.open || active() {
            return false;
        }
        let Some(id) = click_control(name) else {
            return false;
        };
        let now = now_ms();
        if self.last_click.is_some_and(|(previous, at)| {
            previous == id && now.wrapping_sub(at) <= CLICK_DEBOUNCE_MS
        }) {
            return false;
        }
        self.last_click = Some((id, now));
        self.error = None;
        if id == PRESERVE {
            self.preserve = !self.preserve;
        } else if id == VIEW {
            self.damage_page = self.damage_page.is_none().then_some(0);
        } else if id == PREV || id == NEXT {
            if let Some(index) = self.damage_page {
                let (count, _) = damage::pages(Airstrike, index);
                if count > 1 {
                    self.damage_page = Some(if id == PREV {
                        (index + count - 1) % count
                    } else {
                        (index + 1) % count
                    });
                }
            }
        } else if self.damage_page.is_some() {
            return false;
        } else if id == ADD && !self.crafts.is_empty() {
            if self.entries.len() == ENTRY_LIMIT {
                self.error = Some(c"PLANE LIST FULL");
                return false;
            }
            self.entries.push(Entry {
                craft: 0,
                loadout: 0,
                count: 1,
                multiplier: 1,
                first_loadout: 0,
                restore_scroll: true,
            });
            let page = (self.entries.len() - 1) / ROWS_PER_PAGE;
            self.restore_counts = true;
            self.page = page;
        } else if id == RESET {
            self.entries.clear();
            self.page = 0;
            self.restore_counts = true;
            self.ground = false;
        } else if id == START {
            self.ground = !self.ground;
        } else if id == PAGE {
            let pages = self.entries.len().div_ceil(ROWS_PER_PAGE);
            if pages > 1 {
                self.page = (self.page + 1) % pages;
                self.restore_counts = true;
            }
        } else if id == LAUNCH {
            if allies != 0 && enemies != 0 {
                self.error = Some(c"ONLY ONE TEAM IS SUPPORTED");
                return false;
            }
            let mut flights = Vec::new();
            for &entry in &self.entries {
                if let Some((craft, loadout)) = selection(&self.crafts, entry) {
                    flights.push(Flight {
                        craft: craft.key.clone(),
                        loadout: loadout.key.clone(),
                        count: entry.count * entry.multiplier,
                    });
                }
            }
            if flights.is_empty() {
                self.error = Some(c"ADD AT LEAST ONE PLANE");
                return false;
            }
            let live = catalog();
            if flights
                .iter()
                .any(|flight| find_selection(&live, &flight.craft, &flight.loadout).is_none())
            {
                self.open();
                self.error = Some(c"AIRCRAFT CHOICES CHANGED");
                return false;
            }
            let friendly = enemies != 0;
            let replay = if self.preserve {
                damage::replay(Airstrike)
            } else {
                Vec::new()
            };
            let destroyed = |slot| {
                replay
                    .iter()
                    .any(|(key, state)| *key == slot && matches!(state, Replay::Destroyed))
            };
            let all_destroyed = if friendly {
                (0..enemies).all(|index| destroyed(Slot::Enemy(index)))
            } else {
                destroyed(Slot::Editor) && (0..allies).all(|index| destroyed(Slot::Ally(index)))
            };
            if all_destroyed {
                self.error = Some(c"NO SURVIVORS - DISABLE PRESERVE DAMAGE");
                return false;
            }
            if encounter::request(Launch {
                flights,
                friendly,
                ground: self.ground,
                preserve: self.preserve,
                replay,
                staged: Vec::new(),
            }) {
                sound::button("Shipworks Airstrike launch");
                return true;
            }
            return false;
        } else if let Some((row, control)) = ROW_CONTROLS
            .iter()
            .enumerate()
            .find_map(|(row, names)| names.iter().position(|&name| name == id).map(|i| (row, i)))
        {
            let index = self.page * ROWS_PER_PAGE + row;
            if control == REMOVE && index < self.entries.len() {
                self.entries.remove(index);
                self.page = self
                    .page
                    .min(self.entries.len().saturating_sub(1) / ROWS_PER_PAGE);
                self.restore_counts = true;
            } else if let Some(entry) = self.entries.get_mut(index) {
                if control == CRAFT && !self.crafts.is_empty() {
                    entry.craft = (entry.craft + 1) % self.crafts.len();
                    entry.loadout = 0;
                    entry.first_loadout = 0;
                    entry.restore_scroll = true;
                } else if let Some(craft) = self.crafts.get(entry.craft) {
                    match control {
                        SCROLL_UP => entry.scroll_loadouts(craft, -1),
                        SCROLL_DOWN => entry.scroll_loadouts(craft, 1),
                        FIRST_LOADOUT..SCROLL_UP => {
                            let chosen = entry.first_loadout + control - FIRST_LOADOUT;
                            if craft.loadouts.get(chosen).is_some() {
                                entry.loadout = chosen;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        sound::button("Shipworks Airstrike control");
        false
    }
}

impl Entry {
    fn scroll_loadouts(&mut self, craft: &Craft, lines: isize) {
        let target = self
            .first_loadout
            .saturating_add_signed(lines)
            .min(craft.loadouts.len().saturating_sub(LOADOUT_LINES));
        self.restore_scroll |= target != self.first_loadout;
        self.first_loadout = target;
    }
}

fn selection(crafts: &[Craft], entry: Entry) -> Option<(&Craft, &Loadout)> {
    let craft = crafts.get(entry.craft)?;
    Some((craft, craft.loadouts.get(entry.loadout)?))
}

fn find_selection(crafts: &[Craft], key: &CStr, loadout: &CStr) -> Option<(usize, usize)> {
    let craft = crafts
        .iter()
        .position(|craft| craft.key.as_c_str() == key)?;
    let arm = crafts
        .get(craft)?
        .loadouts
        .iter()
        .position(|arm| arm.key.as_c_str() == loadout)?;
    Some((craft, arm))
}

/// Game-thread borrow of the same all-parts library used by DefinePart.
unsafe fn catalog() -> Vec<Craft> {
    let Some((crafts, arms)) = plane_loadouts::airstrike_table() else {
        return Vec::new();
    };
    let library = read::<usize>(ALL_PARTS);
    let find = diag::rd::<usize>(library)
        .and_then(|vtable| diag::rd::<usize>(vtable.wrapping_add(0xc8)))
        .filter(|&find| find != 0);
    let Some(find) = find else {
        return Vec::new();
    };
    let find: unsafe extern "C" fn(usize, *const i8) -> usize = std::mem::transmute(find);
    crafts
        .into_iter()
        .filter_map(|craft| {
            let key = CString::new(craft.name.as_str()).ok()?;
            // SAFETY: this is the verified library's synchronous lookup. Inspect only a
            // borrowed Body template whose class and aircraft logic match the native spawner.
            let part = find(library, key.as_ptr());
            if !diag::readable_span(part, size_of::<Body>())
                || read::<u64>(part + NODE_CODE) != 0xf
                || read::<u32>(part + offset_of!(Body, logic)) != 500
            {
                return None;
            }
            let loadouts: Vec<_> = arms
                .iter()
                .filter(|arm| arm.craft == craft.name)
                .filter_map(|arm| {
                    Some(Loadout {
                        key: CString::new(arm.key.as_str()).ok()?,
                        label: CString::new(arm.airstrike_label()).ok()?,
                    })
                })
                .collect();
            if loadouts.is_empty() {
                return None;
            }
            // Atlas animation names (not the object stats' part previews).
            let icon = match key.as_c_str() {
                key if key == c"CRAFT_T7" => c"sign_T7",
                key if key == c"CRAFT_LA29" => c"sign_LA29",
                _ => c"sign_shell_drone", // Midground1: the tiny aircraft silhouette.
            };
            Some(Craft {
                key,
                label: CString::new(craft.label).ok()?,
                icon,
                loadouts,
            })
        })
        .collect()
}

fn click_control(name: &str) -> Option<&'static CStr> {
    HEADER_CONTROLS
        .into_iter()
        .filter(|&id| id != STATUS && id != RESULT)
        // The native sliders own their own input, including drag interactions.
        .chain(ROW_CONTROLS.into_iter().flat_map(|row| {
            row.into_iter().enumerate().filter_map(|(index, id)| {
                (!matches!(index, FINE | MULTIPLIER | SCROLL_BAR)).then_some(id)
            })
        }))
        .find(|id| id.to_bytes() == name.as_bytes())
}

pub(super) fn is_control(name: &str) -> bool {
    click_control(name).is_some()
}

pub(super) unsafe fn hide(listed: &[usize]) {
    for (name, gui) in HEADER_CONTROLS.into_iter().zip(&HEADER_GUIS) {
        hide_control(name.to_bytes_with_nul(), gui, listed);
    }
    for (names, guis) in ROW_CONTROLS.iter().zip(&ROW_GUIS) {
        hide_row(listed, names, guis);
    }
    clear_overlays();
    damage::preview(Airstrike, None);
}

unsafe fn hide_row(
    listed: &[usize],
    names: &[&CStr; GROUP_CONTROLS],
    guis: &[AtomicUsize; GROUP_CONTROLS],
) {
    for (name, gui) in names.iter().zip(guis) {
        hide_control(name.to_bytes_with_nul(), gui, listed);
    }
}

fn clear_overlays() {
    if let Ok(mut frame) = OVERLAYS.try_lock() {
        frame.fill([None; 4]);
    }
}

/// Draw silhouettes, selection highlights, and scrollbars in the editor label pass.
pub(super) unsafe fn render_overlays() {
    let Some(frame) = OVERLAYS.try_lock().ok().map(|frame| *frame) else {
        return;
    };
    draw_sprites(frame.into_iter().flatten().flatten());
}
