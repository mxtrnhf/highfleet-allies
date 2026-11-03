//! Debug overlay for the tactical battle: rings around every ship and its live AI state above it.
//!
//! Enabled with `"debug_overlay": true` in `ally.json` (default off). The drawing runs from the
//! per-frame battle HUD pass `0x14029c5c0` (`afterBattleStars??`, also the native target-box pass),
//! which is wrapped by [`crate::ally_reticles`] at the call site `0x14029a8c1`. That pass owns the
//! battle camera projection, so every label and ring is computed from the same world-to-screen
//! function (`0x1401adfc0`) and follows the ship while the camera moves.
//!
//! Native details (Steam 1.163):
//! * ships are the global creature vector `0x143faa5d8..0x143faa5e0`, stride 0x18, pointer at +8;
//! * `ship + 0x1840/0x1844` is the hull centre of mass used by the AI;
//! * `ship + 0xad4` is the side float, `+0xd70` the role (1 = flown by the player) and
//!   `+0xbdc` the AI driver state (table in `hf_notes/docs/findings/ai/ai-behaviour-states.md`);
//! * rings use the native screen-space circle drawer `Gfx_DrawCircleClipped` (`0x140244b00`), which
//!   builds the circle from `Gfx_DrawWorldLine` segments and clips it to the screen;
//! * the PLAYER label uses `flash_large_shadow`, and the overlay uses the same font.

use std::{
    ffi::{c_char, CStr},
    fmt::Write as _,
    sync::atomic::{AtomicBool, Ordering},
};

const HUD_ENABLED: usize = 0x1_43a1_1961;
/// `(Vec2* point, bool absolute, float zoom)`.
const WORLD_TO_SCREEN: usize = 0x1_401a_dfc0;
/// `Screen_FloorIfGridEnabled(Vec2*)`.
const FLOOR_POINT: usize = 0x1_402e_51c0;
/// Native screen-space circle drawer: `(centre, radius, ARGB, zoomScale, dashed, style, width, clipMin, clipMax)`.
const DRAW_CIRCLE: usize = 0x1_4024_4b00;
const MEDIA_RES: usize = 0x1_43a1_19e8;

const GET_FONT: usize = 0x1_4017_03e0;
const STYLE_FONT: usize = 0x1_401a_8710;
const FONT_RENDER: usize = 0x1_4016_eb40;
/// The pre-combat PLAYER label font (same recipe as [`crate::draw_marker_text`]).
const FONT_NAME: &CStr = c"flash_large_shadow";
const FONT_SCALE: f32 = 1.0;
const FONT_COLOR: u32 = 0xffe0_e0db;
const FONT_OFFSET_Y: f32 = -46.0;
const FONT_LINE_SPACING: f32 = 14.0;

/// Concentric rings in WORLD units, so they shrink and grow with the battle camera.
const RING_RADII: [f32; 3] = [250.0, 500.0, 1000.0];
/// The rings use the game's friendly cyan / hostile red / neutral lime; alpha makes outer rings lighter.
const RING_ALPHA: [u32; 3] = [0x90ff_ffff, 0x60ff_ffff, 0x38ff_ffff];
const FRIENDLY_RGB: u32 = 0x00e4_ff;
const HOSTILE_RGB: u32 = 0xdd27_27;
const NEUTRAL_RGB: u32 = 0xa8fc_00;
/// One world unit projected by this distance gives the world-to-screen scale.
const SCALE_REFERENCE_WORLD: f32 = 100.0;
const MIN_RING_PIXELS: f32 = 3.0;
const MAX_RING_PIXELS: f32 = 4000.0;
/// Ring-distance text sits just right of the ring line; tiny rings get no text.
const RING_LABEL_DX: f32 = 4.0;
const MIN_RING_LABEL_PIXELS: f32 = 30.0;
/// The AI waypoint (`+0xbf0/+0xbf4`) marker and its dotted lead-in.
const OFF_WAYPOINT_X: usize = 0xbf0;
const OFF_WAYPOINT_Y: usize = 0xbf4;
const WAYPOINT_COLOR: u32 = 0xffff_aa00;
const WAYPOINT_LINE_COLOR: u32 = 0x78ff_aa00;
const WAYPOINT_STEPS: usize = 16;
/// Keep the draw bounded if the native creature vector header is corrupt (the mod allows up to 127 enemies).
const MAX_SHIPS: usize = 256;

static ENABLED: AtomicBool = AtomicBool::new(false);

type WorldToScreenFn = unsafe extern "win64" fn(*mut f32, u8, f32);
type FloorPointFn = unsafe extern "win64" fn(*mut f32);
type DrawCircleFn =
    unsafe extern "win64" fn(*const f32, f32, u32, u8, u8, u32, i32, *const f32, *const f32);
type GetFontFn = unsafe extern "win64" fn(*mut u8, *const c_char) -> *mut u8;
type StyleFn = unsafe extern "win64" fn(*const c_char, f32, i32, u32) -> *mut u8;
type RenderTextFn = unsafe extern "win64" fn(
    *mut u8,
    f32,
    f32,
    u32,
    *const c_char,
    *mut f32,
    *mut f32,
    i32,
    f32,
) -> f32;

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

unsafe fn rd<T: Copy>(address: usize) -> T {
    (address as *const T).read_unaligned()
}

/// Short label for an hf_ai tactics decision.
fn tactics_name(code: u8) -> &'static str {
    match code {
        crate::debug_state::TACTICS_PANIC => "PANIC",
        crate::debug_state::TACTICS_DIVE => "DIVE",
        crate::debug_state::TACTICS_DIVE_SKIP => "DIVE-SKIP",
        crate::debug_state::TACTICS_AIM => "AIM",
        crate::debug_state::TACTICS_STOCK => "STOCK",
        _ => "?",
    }
}

/// Short label for an hf_ai ai_flares mode.
fn flare_name(code: u8) -> &'static str {
    match code {
        crate::debug_state::FLARES_BURST => "FLARES",
        crate::debug_state::FLARES_EVADE => "EVADE",
        crate::debug_state::FLARES_PANIC => "PANIC",
        crate::debug_state::FLARES_ROLL => "ROLL",
        crate::debug_state::FLARES_CURTAIN => "CURTAIN",
        _ => "",
    }
}

/// Short label for the driver state `+0xbdc`.
fn state_name(state: i32) -> &'static str {
    match state {
        0 => "HOLD",
        1 => "ENGAGE",
        2 => "ENGAGE*",
        3 => "MOVE",
        4 => "LEAVE",
        5 => "PARKED",
        6 => "DEAD",
        7 => "ALARM",
        _ => "?",
    }
}

/// Side/role prefix: P = flown by the player, A = friendly AI, E = enemy, N = neutral.
unsafe fn side_tag(ship: usize) -> char {
    if rd::<i32>(ship + crate::OFF_MODE_D70) == 1 {
        return 'P';
    }
    let side = rd::<f32>(ship + crate::OFF_SIDE);
    if side > 0.0 {
        'A'
    } else if side < 0.0 {
        'E'
    } else {
        'N'
    }
}

/// Fixed-size, allocation-free text builder for one label.
struct LineBuf {
    bytes: [u8; 32],
    len: usize,
}

impl LineBuf {
    fn new() -> Self {
        Self { bytes: [0; 32], len: 0 }
    }

    fn finish(mut self) -> Self {
        if self.len < self.bytes.len() {
            self.bytes[self.len] = 0;
        } else {
            self.bytes[self.bytes.len() - 1] = 0;
        }
        self
    }

    fn as_ptr(&self) -> *const c_char {
        self.bytes.as_ptr() as *const c_char
    }
}

impl std::fmt::Write for LineBuf {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        for byte in text.bytes() {
            if self.len + 1 >= self.bytes.len() {
                return Ok(());
            }
            if byte.is_ascii_graphic() || byte == b' ' {
                self.bytes[self.len] = byte;
                self.len += 1;
            }
        }
        Ok(())
    }
}

unsafe fn project(point: &mut [f32; 2]) {
    let to_screen: WorldToScreenFn = std::mem::transmute(WORLD_TO_SCREEN);
    let floor_point: FloorPointFn = std::mem::transmute(FLOOR_POINT);
    to_screen(point.as_mut_ptr(), 0, 1.0);
    floor_point(point.as_mut_ptr());
}

/// Screen-space ring, 1 pixel wide. `zoom_scale` is 0 because the radius is already in pixels.
unsafe fn draw_ring(x: f32, y: f32, radius: f32, argb: u32) {
    let centre = [x, y];
    let clip_min = [f32::MIN, f32::MIN];
    let clip_max = [f32::MAX, f32::MAX];
    let draw: DrawCircleFn = std::mem::transmute(DRAW_CIRCLE);
    draw(centre.as_ptr(), radius, argb, 0, 0, 3, 1, clip_min.as_ptr(), clip_max.as_ptr());
}

unsafe fn draw_text(font: *mut u8, x: f32, y: f32, align: u32, text: &LineBuf) {
    let render: RenderTextFn = std::mem::transmute(FONT_RENDER);
    render(font, x, y, align, text.as_ptr(), std::ptr::null_mut(), std::ptr::null_mut(), 0, 0.0);
}

/// Rings and their world-unit labels.
unsafe fn draw_rings(centre: [f32; 2], pixels_per_world: f32, rgb: u32, font: *mut u8) {
    for (index, radius_world) in RING_RADII.iter().enumerate() {
        let radius_pixels = radius_world * pixels_per_world;
        if !(MIN_RING_PIXELS..=MAX_RING_PIXELS).contains(&radius_pixels) {
            continue;
        }
        let argb = (RING_ALPHA[index] & 0xff00_0000) | rgb;
        draw_ring(centre[0], centre[1], radius_pixels, argb);
        if radius_pixels < MIN_RING_LABEL_PIXELS {
            continue;
        }
        let label_x = centre[0] + radius_pixels + RING_LABEL_DX;
        if label_x > 1900.0 {
            continue;
        }
        let mut text = LineBuf::new();
        let _ = write!(text, "{:.0}u", radius_world);
        draw_text(font, label_x, centre[1] - 7.0, 0, &text.finish());
    }
}

/// Native driver state and hf_ai's extra layer. Returns the y for the line after the last drawn one.
unsafe fn draw_ship_labels(ship: usize, centre: [f32; 2], y: f32, role: i32, font: *mut u8) -> f32 {
    let state = rd::<i32>(ship + crate::OFF_STATE);
    let mut text = LineBuf::new();
    let _ = write!(text, "{} {} {}", side_tag(ship), state, state_name(state));
    draw_text(font, centre[0], y, 2, &text.finish());
    let mut next_y = y + FONT_LINE_SPACING;

    // hf_ai's own layer: flares/evasion override the tactics stage while active. The player ship is not
    // flown by hf_ai, so a leftover entry for it is not shown.
    let now = rd::<f64>(crate::CLOCK);
    if role != 1 {
        if let Some((tactics, flares)) = crate::debug_state::lookup(ship, now) {
            let name = if flares != crate::debug_state::FLARES_NONE {
                flare_name(flares)
            } else {
                tactics_name(tactics)
            };
            let mut line = LineBuf::new();
            let _ = write!(line, "AI {name}");
            draw_text(font, centre[0], next_y, 2, &line.finish());
            next_y += FONT_LINE_SPACING;
        }
    }
    next_y
}

unsafe fn draw_dotted_line(from: [f32; 2], to: [f32; 2], argb: u32) {
    for step in 1..WAYPOINT_STEPS {
        let t = step as f32 / WAYPOINT_STEPS as f32;
        let x = from[0] + (to[0] - from[0]) * t;
        let y = from[1] + (to[1] - from[1]) * t;
        draw_ring(x, y, 1.5, argb);
    }
}

/// The AI's steering waypoint (`+0xbf0/+0xbf4`): a dotted lead-in and a target marker when on screen,
/// otherwise its distance in the label column.
unsafe fn draw_waypoint(
    ship: usize,
    world: [f32; 2],
    centre: [f32; 2],
    role: i32,
    label_y: f32,
    font: *mut u8,
) {
    if role == 1 {
        return;
    }
    let waypoint = [rd::<f32>(ship + OFF_WAYPOINT_X), rd::<f32>(ship + OFF_WAYPOINT_Y)];
    if !waypoint[0].is_finite() || !waypoint[1].is_finite() {
        return;
    }
    let distance = (waypoint[0] - world[0]).hypot(waypoint[1] - world[1]);
    if !distance.is_finite() || distance < 10.0 {
        return;
    }

    let mut screen = waypoint;
    project(&mut screen);
    if !screen[0].is_finite() || !screen[1].is_finite() {
        return;
    }
    let on_screen = (-100.0..=2020.0).contains(&screen[0]) && (-100.0..=1180.0).contains(&screen[1]);
    if !on_screen {
        let mut text = LineBuf::new();
        let _ = write!(text, "DEST {distance:.0}u");
        draw_text(font, centre[0], label_y, 2, &text.finish());
        return;
    }

    draw_dotted_line(centre, screen, WAYPOINT_LINE_COLOR);
    draw_ring(screen[0], screen[1], 4.0, WAYPOINT_COLOR);
    draw_ring(screen[0], screen[1], 9.0, WAYPOINT_COLOR & 0x60ff_ffff);
    let mut text = LineBuf::new();
    let _ = write!(text, "WP {distance:.0}u");
    draw_text(font, screen[0], (screen[1] - 18.0).max(8.0), 2, &text.finish());
}

/// Draws rings, state labels and the AI waypoint for one live ship. Returns false when off-screen.
unsafe fn draw_ship(ship: usize, font: *mut u8) -> bool {
    let world = [rd::<f32>(ship + 0x1840), rd::<f32>(ship + 0x1844)];
    if !world[0].is_finite() || !world[1].is_finite() {
        return false;
    }

    let mut centre = world;
    let mut reference = [world[0] + SCALE_REFERENCE_WORLD, world[1]];
    project(&mut centre);
    project(&mut reference);
    if !centre[0].is_finite() || !centre[1].is_finite() {
        return false;
    }
    if !(-100.0..=2020.0).contains(&centre[0]) || !(-100.0..=1180.0).contains(&centre[1]) {
        return false;
    }

    let dx = reference[0] - centre[0];
    let dy = reference[1] - centre[1];
    let pixels_per_world = (dx * dx + dy * dy).sqrt() / SCALE_REFERENCE_WORLD;
    if !pixels_per_world.is_finite() || pixels_per_world <= 0.0 {
        return false;
    }

    let side = rd::<f32>(ship + crate::OFF_SIDE);
    let rgb = if side > 0.0 {
        FRIENDLY_RGB
    } else if side < 0.0 {
        HOSTILE_RGB
    } else {
        NEUTRAL_RGB
    };
    draw_rings(centre, pixels_per_world, rgb, font);

    let role = rd::<i32>(ship + crate::OFF_MODE_D70);
    let label_y = (centre[1] + FONT_OFFSET_Y).max(8.0);
    let next_y = draw_ship_labels(ship, centre, label_y, role, font);
    draw_waypoint(ship, world, centre, role, next_y, font);
    true
}

/// Runs after the native target/spawn box pass, once per battle frame.
pub unsafe fn draw() {
    if !ENABLED.load(Ordering::Relaxed) || rd::<u8>(HUD_ENABLED) == 0 {
        return;
    }
    let manager = rd::<usize>(MEDIA_RES) as *mut u8;
    if manager.is_null() {
        return;
    }
    let get_font: GetFontFn = std::mem::transmute(GET_FONT);
    let font = get_font(manager, FONT_NAME.as_ptr());
    if font.is_null() {
        return;
    }
    let base = font as usize;
    let (old_scale, old_color, old_blend): (f32, u32, i32) =
        (rd(base + 0x100c), rd(base + 0x1020), rd(base + 0x1028));
    let style: StyleFn = std::mem::transmute(STYLE_FONT);
    style(FONT_NAME.as_ptr(), FONT_SCALE, 2, FONT_COLOR);

    let (begin, end): (usize, usize) = (rd(crate::CREATURES_BEGIN), rd(crate::CREATURES_END));
    if begin != 0 && end > begin {
        let entries = ((end - begin) / 0x18).min(crate::CREATURE_LIST_LIMIT).min(MAX_SHIPS);
        for index in 0..entries {
            let ship = rd::<usize>(begin + index * 0x18 + 8);
            if !crate::live_ship(ship) {
                continue;
            }
            draw_ship(ship, font);
        }
    }

    style(FONT_NAME.as_ptr(), old_scale, old_blend, old_color);
}
