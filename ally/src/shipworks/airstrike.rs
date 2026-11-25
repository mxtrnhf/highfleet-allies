//! Standalone Shipworks Airstrike panel. The encounter adapter never uses the
//! player's inventory; this panel copies choices from the live aircraft tables.

mod encounter;

use std::{
    ffi::{CStr, CString},
    mem::{offset_of, size_of},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use crate::{
    diag,
    native::read,
    native_layout::{Body, EscadraString, Gui, PartStats, NODE_CODE},
    plane_loadouts, precombat, sound,
    time::now_ms,
};

use super::{
    hide_control, native_string, show_control, write, ControlGeometry, BUTTON_GAP, BUTTON_STYLE,
    CLICK_DEBOUNCE_MS, ENEMY_ROW_STYLE,
};

pub(super) use encounter::{
    active, cancel_request, cancel_unhandled_request, install, leave, load_fight, requested,
};

pub(super) const BUTTON: &CStr = c"SW_ACE_AIR";
pub(super) const LAUNCH: &CStr = c"SW_AIR_LAUNCH";
pub(super) const LEVEL: &CStr = c"Bombers.seria";
const ADD: &CStr = c"SW_AIR_ADD";
const RESET: &CStr = c"SW_AIR_RESET";
const START: &CStr = c"SW_AIR_START";
const STATUS: &CStr = c"SW_AIR_STATUS";
const PAGE: &CStr = c"SW_AIR_PAGE";
const HEADER_CONTROLS: [&CStr; 6] = [ADD, RESET, START, LAUNCH, STATUS, PAGE];
static HEADER_GUIS: [AtomicUsize; HEADER_CONTROLS.len()] =
    [const { AtomicUsize::new(0) }; HEADER_CONTROLS.len()];

const ROWS_PER_PAGE: usize = 4;
const ENTRY_LIMIT: usize = 8;
const MAX_PLANES: u8 = 15;
const ROW_CONTROLS: [[&CStr; 3]; ROWS_PER_PAGE] = [
    [c"SW_AIR_CRAFT1", c"SW_AIR_LOAD1", c"SW_AIR_COUNT1"],
    [c"SW_AIR_CRAFT2", c"SW_AIR_LOAD2", c"SW_AIR_COUNT2"],
    [c"SW_AIR_CRAFT3", c"SW_AIR_LOAD3", c"SW_AIR_COUNT3"],
    [c"SW_AIR_CRAFT4", c"SW_AIR_LOAD4", c"SW_AIR_COUNT4"],
];
static ROW_GUIS: [[AtomicUsize; 3]; ROWS_PER_PAGE] =
    [const { [const { AtomicUsize::new(0) }; 3] }; ROWS_PER_PAGE];

const FRAME_WIDTH: usize = 0x1_43a1_58fc;
const FRAME_HEIGHT: usize = 0x1_43a1_3e5c;
const PANEL_MARGIN: f32 = 36.0;
const ROW_GAP: f32 = 6.0;
const SLIDER_SIZE: usize = 0xea8;
const SLIDER_CTOR: usize = 0x1_4012_b4a0;
const SLIDER_INIT: usize = 0x1_4016_b9c0;
const SLIDER_SET_VALUE: usize = 0x1_4016_bf00;
const SLIDER_RENDER_VALUE: usize = 0xe70;
const SLIDER_IS_FLOAT: usize = 0xe71;
const NATIVE_NEW: usize = 0x1_4033_4094;
const GET_STATS: usize = 0x1_4028_1e00;
// Stats::populateObjects (0x1402821c0) registers m_icon_name after the OID.
const STATS_ICON: usize = offset_of!(PartStats, unknown_0048h) + size_of::<EscadraString>();
const ALL_PARTS: usize = 0x1_43a1_39f0;

const MEDIA: usize = 0x1_43a1_19e8;
const GET_ANIMATION: usize = 0x1_4017_0230;
const HGE_RENDERER: usize = 0x1_439e_8670;
const GFX_RENDER_QUAD: usize = 0x1_4016_cc80;
const VERTEX_SIZE: usize = 0x58;
const QUAD_TEXTURE: usize = 0x160;
const QUAD_BLEND: usize = 0x168;
const QUAD_SIZE: usize = 0x170;
const ANIMATION_DIMENSIONS: usize = 0x188;
const COLOR_ADD: u32 = 1;
const ALPHA_BLEND: u32 = 2;

struct Craft {
    key: CString,
    label: CString,
    icon: Arc<CStr>,
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
}

pub(super) struct Panel {
    open: bool,
    crafts: Vec<Craft>,
    entries: Vec<Entry>,
    page: usize,
    restore_counts: bool,
    ground: bool,
    error: Option<&'static CStr>,
    last_click: Option<(&'static CStr, u64)>,
}

struct Icon {
    name: Arc<CStr>,
    position: [f32; 2],
    size: f32,
}

static ICONS: Mutex<[Option<Icon>; ROWS_PER_PAGE]> = Mutex::new([const { None }; ROWS_PER_PAGE]);

impl Panel {
    pub(super) const fn new() -> Self {
        Self {
            open: false,
            crafts: Vec::new(),
            entries: Vec::new(),
            page: 0,
            restore_counts: true,
            ground: false,
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
        self.error = None;
        clear_icons();
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
        show(0, c"ADD PLANE", cell(0.0, 0.55, top), BUTTON_STYLE);
        show(1, c"RESET", cell(0.55, 1.0, top), BUTTON_STYLE);
        let step = height + ROW_GAP;
        let first = self.page * ROWS_PER_PAGE;
        let mut y = top + step;
        let mut icons = [const { None }; ROWS_PER_PAGE];
        for (index, ((names, guis), icon)) in ROW_CONTROLS
            .iter()
            .zip(&ROW_GUIS)
            .zip(&mut icons)
            .enumerate()
        {
            let Some(entry) = self.entries.get_mut(first + index) else {
                hide_row(listed, names, guis);
                continue;
            };
            let Some((craft, loadout)) = selection(&self.crafts, *entry) else {
                hide_row(listed, names, guis);
                continue;
            };
            for (control, label, area) in [
                (0, &craft.label, cell(0.12, 0.49, y)),
                (1, &loadout.label, cell(0.0, 1.0, y + step)),
            ] {
                show_control(
                    names[control].to_bytes_with_nul(),
                    &guis[control],
                    listed,
                    template,
                    label,
                    area,
                    BUTTON_STYLE,
                );
            }
            show_slider(
                names[2],
                &guis[2],
                listed,
                template,
                cell(0.5, 1.0, y),
                entry,
                self.restore_counts,
            );
            *icon = Some(Icon {
                name: Arc::clone(&craft.icon),
                position: [
                    left + panel_width * 0.06 + frame_width * 0.5,
                    y + frame_height * 0.5,
                ],
                size: (height * 1.3).min(panel_width * 0.12),
            });
            y += 2.0 * step + ROW_GAP;
        }
        if let Ok(mut frame) = ICONS.try_lock() {
            *frame = icons;
        }
        self.restore_counts = false;

        let pages = self.entries.len().div_ceil(ROWS_PER_PAGE);
        if pages > 1 {
            if let Ok(page) = CString::new(format!("PAGE {} / {}  >", self.page + 1, pages)) {
                show(5, &page, cell(0.0, 1.0, y), BUTTON_STYLE);
            }
            y += step;
        } else {
            hide_control(PAGE.to_bytes_with_nul(), &HEADER_GUIS[5], listed);
        }
        show(
            2,
            if self.ground { c"GROUND" } else { c"AIRBORNE" },
            cell(0.0, 0.5, y),
            BUTTON_STYLE,
        );
        show(3, c"LAUNCH", cell(0.5, 1.0, y), BUTTON_STYLE);
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
    pub(super) unsafe fn click(&mut self, name: &str, allies: bool, enemies: bool) -> bool {
        if !self.open || active() {
            return false;
        }
        let Some(id) = control(name) else {
            return false;
        };
        // The control is a native Slider; allow its input handler to own drag interactions.
        if ROW_CONTROLS.iter().any(|row| id == row[2]) {
            return false;
        }
        let now = now_ms();
        if self.last_click.is_some_and(|(previous, at)| {
            previous == id && now.wrapping_sub(at) <= CLICK_DEBOUNCE_MS
        }) {
            return false;
        }
        self.last_click = Some((id, now));
        self.error = None;
        if id == ADD && !self.crafts.is_empty() {
            if self.entries.len() == ENTRY_LIMIT {
                self.error = Some(c"PLANE LIST FULL");
                return false;
            }
            self.entries.push(Entry {
                craft: 0,
                loadout: 0,
                count: 1,
            });
            let page = (self.entries.len() - 1) / ROWS_PER_PAGE;
            self.restore_counts |= self.page != page;
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
            if allies && enemies {
                self.error = Some(c"ONLY ONE TEAM IS SUPPORTED");
                return false;
            }
            let mut flights = Vec::new();
            for &entry in &self.entries {
                if entry.count == 0 {
                    continue;
                }
                if let Some((craft, loadout)) = selection(&self.crafts, entry) {
                    flights.push(Flight {
                        craft: craft.key.clone(),
                        loadout: loadout.key.clone(),
                        count: entry.count,
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
            if encounter::request(Launch {
                flights,
                friendly: enemies,
                ground: self.ground,
            }) {
                sound::button("Shipworks Airstrike launch");
                return true;
            }
            return false;
        } else if let Some((row, names)) = ROW_CONTROLS
            .iter()
            .enumerate()
            .find(|(_, names)| names.contains(&id))
        {
            if let Some(entry) = self.entries.get_mut(self.page * ROWS_PER_PAGE + row) {
                if id == names[0] && !self.crafts.is_empty() {
                    entry.craft = (entry.craft + 1) % self.crafts.len();
                    entry.loadout = 0;
                } else if let Some(craft) = self.crafts.get(entry.craft) {
                    entry.loadout = (entry.loadout + 1) % craft.loadouts.len();
                }
            }
        }
        sound::button("Shipworks Airstrike control");
        false
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
    let get_stats: unsafe extern "C" fn(i32) -> usize = std::mem::transmute(GET_STATS);
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
                        label: CString::new(arm.label.as_str()).ok()?,
                    })
                })
                .collect();
            if loadouts.is_empty() {
                return None;
            }
            let stats = get_stats(read::<i32>(part + offset_of!(Body, part_index)));
            let icon = native_string(stats + STATS_ICON)
                .filter(|icon| !icon.to_bytes().is_empty())
                .unwrap_or_else(|| key.clone());
            Some(Craft {
                key,
                label: CString::new(craft.label).ok()?,
                icon: Arc::from(icon),
                loadouts,
            })
        })
        .collect()
}

fn control(name: &str) -> Option<&'static CStr> {
    HEADER_CONTROLS
        .into_iter()
        .filter(|&id| id != STATUS)
        .chain(ROW_CONTROLS.into_iter().flatten())
        .find(|id| id.to_bytes() == name.as_bytes())
}

pub(super) fn is_control(name: &str) -> bool {
    control(name).is_some_and(|id| !ROW_CONTROLS.iter().any(|row| id == row[2]))
}

pub(super) unsafe fn hide(listed: &[usize]) {
    for (name, gui) in HEADER_CONTROLS.into_iter().zip(&HEADER_GUIS) {
        hide_control(name.to_bytes_with_nul(), gui, listed);
    }
    for (names, guis) in ROW_CONTROLS.iter().zip(&ROW_GUIS) {
        hide_row(listed, names, guis);
    }
    clear_icons();
}

unsafe fn hide_row(listed: &[usize], names: &[&CStr; 3], guis: &[AtomicUsize; 3]) {
    for (name, gui) in names.iter().zip(guis) {
        hide_control(name.to_bytes_with_nul(), gui, listed);
    }
}

fn clear_icons() {
    if let Ok(mut icons) = ICONS.try_lock() {
        icons.fill_with(|| None);
    }
}

unsafe fn show_slider(
    name: &CStr,
    slot: &AtomicUsize,
    listed: &[usize],
    template: usize,
    area: ControlGeometry,
    entry: &mut Entry,
    restore: bool,
) {
    let visible = precombat::listed_shown(
        listed,
        slot.load(Ordering::Relaxed),
        name.to_bytes_with_nul(),
    );
    let Some((gui, created)) =
        precombat::get_or_make_with(name.to_bytes_with_nul(), slot, listed, true, || {
            make_slider(name, area, template)
        })
    else {
        return;
    };
    precombat::set_gui_active(gui, true);
    if !created {
        precombat::move_gui_rect(gui, area.position, area.dimensions);
    }
    if restore || !visible || created {
        let set: unsafe extern "C" fn(usize, f32) = std::mem::transmute(SLIDER_SET_VALUE);
        set(gui, f32::from(entry.count));
    } else {
        let value = read::<f32>(gui + offset_of!(Gui, value));
        if value.is_finite() {
            entry.count = value.round().clamp(0.0, f32::from(MAX_PLANES)) as u8;
        }
    }
}

unsafe fn make_slider(name: &CStr, area: ControlGeometry, template: usize) -> *mut u8 {
    // SAFETY: duplicates Shipworks' native Slider recipe (0x1400ceade). Allocate
    // and initialize on the GUI thread; the root then owns its strings and art.
    let new: unsafe extern "C" fn(usize) -> *mut u8 = std::mem::transmute(NATIVE_NEW);
    let ctor: unsafe extern "C" fn(*mut u8) -> *mut u8 = std::mem::transmute(SLIDER_CTOR);
    let init: unsafe extern "C" fn(
        *mut u8,
        *const i8,
        f32,
        f32,
        f32,
        f32,
        *const i8,
        f32,
        f32,
        *const i8,
        *const i8,
    ) = std::mem::transmute(SLIDER_INIT);
    let storage = new(SLIDER_SIZE);
    if storage.is_null() {
        return storage;
    }
    let gui = ctor(storage);
    precombat::attach_gui(gui);
    init(
        gui,
        name.as_ptr(),
        area.position[0],
        area.position[1],
        area.dimensions[0],
        area.dimensions[1],
        c"".as_ptr(),
        0.0,
        f32::from(MAX_PLANES),
        c"slider_green".as_ptr(),
        c"slider_std".as_ptr(),
    );
    let address = gui as usize;
    write(
        address + offset_of!(Body, stage),
        read::<i32>(template + offset_of!(Body, stage)),
    );
    write(
        address + offset_of!(Body, layer),
        read::<i32>(template + offset_of!(Body, layer)),
    );
    write(address + offset_of!(Body, focusable), 1u8);
    write(
        address + offset_of!(Gui, text_color),
        read::<u32>(template + offset_of!(Gui, text_color)),
    );
    write(address + SLIDER_RENDER_VALUE, 1u8);
    write(address + SLIDER_IS_FLOAT, 0u8);
    precombat::load_gui(address);
    gui
}

/// Draw the actual media resource using the Default shader's COLORADD filter.
/// White saturates RGB on the editor's UNORM target; `ps_add` keeps texture
/// alpha. Only the POD draw packet is copied; the shared animation isn't edited.
pub(super) unsafe fn render_icons() {
    let Ok(icons) = ICONS.try_lock() else {
        return;
    };
    if icons.iter().all(Option::is_none) {
        return;
    }
    let Some(media) = diag::rd::<usize>(MEDIA).filter(|&ptr| ptr != 0) else {
        return;
    };
    let Some(renderer) = diag::rd::<usize>(HGE_RENDERER).filter(|&ptr| ptr != 0) else {
        return;
    };
    let animation: unsafe extern "C" fn(usize, *const i8) -> usize =
        std::mem::transmute(GET_ANIMATION);
    let render: unsafe extern "C" fn(usize, usize) = std::mem::transmute(GFX_RENDER_QUAD);
    for icon in icons.iter().flatten() {
        // SAFETY: the editor label pass uses the Default shader; animation and
        // texture remain live for the synchronous submit, whose vertices are copied.
        let source = animation(media, icon.name.as_ptr());
        if !diag::readable_span(source, ANIMATION_DIMENSIONS + 8)
            || read::<usize>(source + QUAD_TEXTURE) == 0
        {
            continue;
        }
        let width = read::<f32>(source + ANIMATION_DIMENSIONS);
        let height = read::<f32>(source + ANIMATION_DIMENSIONS + 4);
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            continue;
        }
        let scale = icon.size / width.max(height);
        let mut quad = [0u64; QUAD_SIZE / size_of::<u64>()];
        let data = quad.as_mut_ptr() as usize;
        std::ptr::copy_nonoverlapping(source as *const u8, data as *mut u8, QUAD_SIZE);
        for (index, [x, y]) in [[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]]
            .into_iter()
            .enumerate()
        {
            let vertex = data + index * VERTEX_SIZE;
            write(vertex, icon.position[0] + x * width * scale);
            write(vertex + 4, icon.position[1] + y * height * scale);
            write(vertex + 8, 0.0f32);
            write(vertex + 0x18, 0xffff_ffffu32);
        }
        write(data + QUAD_BLEND, COLOR_ADD | ALPHA_BLEND);
        render(renderer, data);
    }
}
