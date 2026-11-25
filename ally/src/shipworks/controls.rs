//! Shared native slider, mouse and overlay primitives for the Shipworks pickers.

use std::{
    ffi::CStr,
    mem::{offset_of, size_of},
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::{
    diag,
    native::read,
    native_layout::{Body, Gui},
    precombat,
};

use super::{write, ControlGeometry, ControlStyle, COPY_STRING, NATIVE_NEW};

const SLIDER_SIZE: usize = 0xea8;
const SLIDER_CTOR: usize = 0x1_4012_b4a0;
const SLIDER_INIT: usize = 0x1_4016_b9c0;
const SLIDER_SET_VALUE: usize = 0x1_4016_bf00;
const SLIDER_RENDER_VALUE: usize = 0xe70;
const SLIDER_IS_FLOAT: usize = 0xe71;
const HGE: usize = 0x1_43a1_19e0;
const GET_MOUSE_POS: usize = 0x1_4017_3870;
const GET_MOUSE_WHEEL: usize = 0x1_4017_3890;
const MEDIA: usize = 0x1_43a1_19e8;
const GET_ANIMATION: usize = 0x1_4017_0230;
const HGE_RENDERER: usize = 0x1_439e_8670;
const GFX_RENDER_QUAD: usize = 0x1_4016_cc80;
const VERTEX_SIZE: usize = 0x58;
const QUAD_TEXTURE: usize = 0x160;
const QUAD_BLEND: usize = 0x168;
const QUAD_SIZE: usize = 0x170;
const ANIMATION_DIMENSIONS: usize = 0x188;
const ALPHA_BLEND: u32 = 2;
const TRACK_COLOR: u32 = 0xff6a_7074;
const PICKER_COLOR: u32 = 0xffe4_d9ac;

pub(super) const PICKER_ROW_STYLE: ControlStyle = ControlStyle {
    button: false,
    clickable: true,
    color: Some(PICKER_COLOR),
};
pub(super) const PICKER_SELECTED_STYLE: ControlStyle = ControlStyle {
    color: Some(0xffa4_d77a),
    ..PICKER_ROW_STYLE
};

/// The complete GUI scan and its validated TEST template for this editor frame.
#[derive(Clone, Copy)]
pub(super) struct EditorControls<'a> {
    pub(super) listed: &'a [usize],
    pub(super) template: usize,
}

#[derive(Clone, Copy)]
pub(super) enum SliderKind {
    Integer { max: u8, suffix: &'static CStr },
    Scroll,
}

impl SliderKind {
    fn bounds(self) -> (f32, f32) {
        match self {
            Self::Integer { max, .. } => (1.0, f32::from(max)),
            Self::Scroll => (0.0, 1.0),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct OverlaySprite {
    name: &'static CStr,
    area: ControlGeometry,
    color: u32,
    keep_aspect: bool,
}

impl OverlaySprite {
    pub(super) fn icon(name: &'static CStr, area: ControlGeometry, frame: [f32; 2]) -> Self {
        Self {
            name,
            area: screen_area(area, frame),
            color: PICKER_COLOR,
            keep_aspect: true,
        }
    }
}

fn screen_area(mut area: ControlGeometry, [width, height]: [f32; 2]) -> ControlGeometry {
    area.position[0] += width * 0.5;
    area.position[1] += height * 0.5;
    area
}

pub(super) fn vertical_slider(mut area: ControlGeometry) -> ControlGeometry {
    // Slider_DragValueToCursor (0x14012e8d0) chooses the longer axis.
    area.dimensions[0] = area.dimensions[0].min(area.dimensions[1] * 0.75);
    area
}

pub(super) fn scrollbar_art(
    mut track: ControlGeometry,
    frame: [f32; 2],
    visible: usize,
    limit: usize,
    first: usize,
) -> [OverlaySprite; 2] {
    let [x, y] = track.position;
    let [width, height] = track.dimensions;
    let visible_fraction = visible as f32 / (limit + visible) as f32;
    let thumb_height = height * visible_fraction.max(0.20);
    let travel = height - thumb_height;
    let thumb = ControlGeometry {
        position: [x, y + travel * (first as f32 / limit as f32 - 0.5)],
        dimensions: [width * 0.75, thumb_height],
    };
    track.dimensions[0] *= 0.56;
    [(track, TRACK_COLOR), (thumb, PICKER_COLOR)].map(|(area, color)| OverlaySprite {
        name: c"white16",
        area: screen_area(area, frame),
        color,
        keep_aspect: false,
    })
}

/// Read the live editor input cached by HGE. GUI positions are screen-centered.
pub(super) unsafe fn wheel_input([width, height]: [f32; 2]) -> Option<([f32; 2], i32)> {
    let hge = read::<usize>(HGE);
    if hge == 0 {
        return None;
    }
    // SAFETY: Shipworks' frame pass runs on the HGE owner thread. These getters
    // read scalar fields only; the position getter fills two local f32s.
    let wheel: unsafe extern "C" fn(usize) -> i32 = std::mem::transmute(GET_MOUSE_WHEEL);
    let delta = wheel(hge);
    if delta == 0 {
        return None;
    }
    let position: unsafe extern "C" fn(usize, *mut f32, *mut f32) =
        std::mem::transmute(GET_MOUSE_POS);
    let (mut x, mut y) = (0.0, 0.0);
    position(hge, &mut x, &mut y);
    (x.is_finite() && y.is_finite()).then_some(([x - width * 0.5, y - height * 0.5], delta))
}

/// Reuse or construct a native slider against the current complete editor scan.
/// The game thread owns the GUI and processes its drag input.
pub(super) unsafe fn show_slider(
    name: &CStr,
    slot: &AtomicUsize,
    controls: EditorControls<'_>,
    area: ControlGeometry,
    value: f32,
    kind: SliderKind,
    restore: bool,
) -> f32 {
    let EditorControls { listed, template } = controls;
    let visible = precombat::listed_shown(
        listed,
        slot.load(Ordering::Relaxed),
        name.to_bytes_with_nul(),
    );
    let Some((gui, created)) =
        precombat::get_or_make_with(name.to_bytes_with_nul(), slot, listed, true, || {
            make_slider(name, area, template, kind)
        })
    else {
        return value;
    };
    precombat::set_gui_active(gui, true);
    if !created {
        precombat::move_gui_rect(gui, area.position, area.dimensions);
    }
    if restore || !visible || created {
        let set: unsafe extern "C" fn(usize, f32) = std::mem::transmute(SLIDER_SET_VALUE);
        set(gui, value);
        value
    } else {
        let native_value = read::<f32>(gui + offset_of!(Gui, value));
        if native_value.is_finite() {
            let (min, max) = kind.bounds();
            native_value.clamp(min, max)
        } else {
            value
        }
    }
}

unsafe fn make_slider(
    name: &CStr,
    area: ControlGeometry,
    template: usize,
    kind: SliderKind,
) -> *mut u8 {
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
    let (min, max) = kind.bounds();
    let (bar, background, suffix) = match kind {
        // The scrollbar slider captures input only; the label pass draws its art.
        SliderKind::Scroll => (c"", c"", c""),
        // The bar art covers the right (unfilled) end of its background.
        SliderKind::Integer { suffix, .. } => (c"slider_std", c"slider_green", suffix),
    };
    init(
        gui,
        name.as_ptr(),
        area.position[0],
        area.position[1],
        area.dimensions[0],
        area.dimensions[1],
        c"".as_ptr(),
        min,
        max,
        bar.as_ptr(),
        background.as_ptr(),
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
    let scroll = matches!(kind, SliderKind::Scroll);
    write(address + SLIDER_RENDER_VALUE, u8::from(!scroll));
    write(address + SLIDER_IS_FLOAT, u8::from(scroll));
    if !suffix.to_bytes().is_empty() {
        let assign: unsafe extern "C" fn(usize, *const i8, usize) =
            std::mem::transmute(COPY_STRING);
        assign(
            address + offset_of!(Gui, note),
            suffix.as_ptr(),
            suffix.to_bytes().len(),
        );
    }
    precombat::load_gui(address);
    gui
}

/// Draw atlas sprites in the editor label pass without editing shared animations.
pub(super) unsafe fn render_overlays(sprites: impl Iterator<Item = OverlaySprite>) {
    let mut sprites = sprites.peekable();
    if sprites.peek().is_none() {
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
    for sprite in sprites {
        // SAFETY: the editor label pass uses the Default shader; animation and
        // texture remain live for the synchronous submit, whose vertices are copied.
        let source = animation(media, sprite.name.as_ptr());
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
        let ControlGeometry {
            position,
            mut dimensions,
        } = sprite.area;
        if sprite.keep_aspect {
            let scale = (dimensions[0] / width).min(dimensions[1] / height);
            dimensions = [width * scale, height * scale];
        }
        let mut quad = [0u64; QUAD_SIZE / size_of::<u64>()];
        let data = quad.as_mut_ptr() as usize;
        std::ptr::copy_nonoverlapping(source as *const u8, data as *mut u8, QUAD_SIZE);
        for (index, [x, y]) in [[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]]
            .into_iter()
            .enumerate()
        {
            let vertex = data + index * VERTEX_SIZE;
            write(vertex, position[0] + x * dimensions[0]);
            write(vertex + 4, position[1] + y * dimensions[1]);
            write(vertex + 8, 0.0f32);
            write(vertex + 0x18, sprite.color);
        }
        write(data + QUAD_BLEND, ALPHA_BLEND);
        render(renderer, data);
    }
}
