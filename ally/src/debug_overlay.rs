//! Debug overlay for the tactical battle: rings around every ship and its live AI state above it.
//!
//! Configured by its own `"debug_overlay"` block in `ally.json` (default off): `enable` is the
//! master switch, `circles` is the ring-radius list in world units (empty = no rings),
//! `ai_waypoint` turns the AI waypoint markers on/off, and `prediction_marker` turns the hostile
//! lead (aim-assist) markers and their "put guns here" bearing circles on/off. The old
//! `logging.debug_overlay` boolean still loads as `enable`. The drawing runs from the per-frame
//! battle HUD pass `0x14029c5c0` (`afterBattleStars??`, also the native target-box pass), which is
//! wrapped by [`crate::ally_reticles`] at the call site `0x14029a8c1`. That pass owns the battle
//! camera projection, so every label and ring is computed from the same world-to-screen
//! function (`0x1401adfc0`) and follows the ship while the camera moves.
//!
//! Native details (Steam 1.163):
//! * ships are the global creature vector `0x143faa5d8..0x143faa5e0`, stride 0x18, pointer at +8;
//! * `Creature::bounds_center_x/y` is the hull bounding-box centre used by the AI;
//! * ship velocity is the root body's `Body::velocity_x/y` (`+0x130/+0x134`);
//! * a gun's loaded ammo is `Body::weapon_ammo` (`+0x850`), an index into the global ammo list
//!   `0x143a13be0` (`Ammo` records, stride 0x188, muzzle `speed` at `+0x158`) — the same fields
//!   the native AI lead reads (hf_notes docs/findings/combat/firing-and-shells.md sections 7-8);
//! * `ship + 0xad4` is the side float, `+0xd70` the role (1 = flown by the player) and
//!   `+0xbdc` the AI driver state (table in `hf_notes/docs/findings/ai/ai-behaviour-states.md`);
//! * rings use the native screen-space circle drawer `Gfx_DrawCircleClipped` (`0x140244b00`), which
//!   builds the circle from `Gfx_DrawWorldLine` segments and clips it to the screen;
//! * the PLAYER label uses `flash_large_shadow`, and the overlay uses the same font.

use std::{
    ffi::{c_char, CStr},
    fmt::Write as _,
    mem::{offset_of, size_of},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

use crate::native_layout::{Ammo, Body, Creature, Link};

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

/// The rings use the game's friendly cyan / hostile red / neutral lime.
const FRIENDLY_RGB: u32 = 0x00e4_ff;
const HOSTILE_RGB: u32 = 0xdd27_27;
const NEUTRAL_RGB: u32 = 0xa8fc_00;
/// One world unit projected by this distance gives the world-to-screen scale.
const SCALE_REFERENCE_WORLD: f32 = 100.0;
/// Ring pixel-size bounds: nothing below 3 or above 4000 screen pixels is drawn.
const MIN_RING_PIXELS: f32 = 3.0;
const MAX_RING_PIXELS: f32 = 4000.0;
/// Ring-distance text sits just right of the ring line; tiny rings get no text.
const RING_LABEL_DX: f32 = 4.0;
const MIN_RING_LABEL_PIXELS: f32 = 30.0;
/// Ring alpha fades half-strength per ring outward, floored so far rings stay visible.
const RING_INNER_ALPHA: u32 = 0x90;
const RING_MIN_ALPHA: u32 = 0x38;
/// Ceiling on configured ring radii; more would drown the battle view.
const MAX_CIRCLES: usize = 8;
/// The AI waypoint (`Creature::waypoint_x/y`) marker and its dotted lead-in.
const OFF_WAYPOINT_X: usize = offset_of!(Creature, waypoint_x);
const OFF_WAYPOINT_Y: usize = offset_of!(Creature, waypoint_y);
const WAYPOINT_COLOR: u32 = 0xffff_aa00;
const WAYPOINT_LINE_COLOR: u32 = 0x78ff_aa00;
const WAYPOINT_STEPS: usize = 16;
/// The gun-group vectors (`Creature::gun_group_0`, then `gun_group_1`): entries are `Link`s (stride
/// 0x18), the gun `Body` at the link target. The native AI lead reads the first gun the same way
/// (firing-and-shells.md section 8).
const OFF_GUN_GROUP_0: usize = offset_of!(Creature, gun_group_0);
const OFF_GUN_GROUP_1: usize = offset_of!(Creature, gun_group_1);
/// Ceiling on the guns read per group; the vector is corrupt above that.
const MAX_GUNS_PER_GROUP: usize = 64;
/// The game's ammo list header (`{begin*, end*}` of `Ammo` records, stride 0x188); the same header
/// `plane_loadouts.rs` reads. A gun's loaded ammo is the index into this list.
const AMMO_LIST: usize = 0x1_43a1_3be0;
/// Ceiling on the records read from the ammo list; a longer list means the header is corrupt.
const AMMO_LIST_LIMIT: usize = 10_000;
/// Lead marker on hostile ships: the game's hostile red, a half-strength halo and the flight time.
/// Each lead marker also draws a small "put guns here" bearing circle just outside the player's
/// own hull, so the gun angle is readable at the ship itself.
const PREDICTION_COLOR: u32 = 0xffff_dd27;
const PREDICTION_HALO_COLOR: u32 = 0x60ff_dd27;
const BEARING_MARGIN_PX: f32 = 12.0;
const BEARING_MIN_PX: f32 = 24.0;
const BEARING_MAX_PX: f32 = 150.0;
const BEARING_RING_PX: f32 = 3.0;
const BEARING_HALO_PX: f32 = 6.0;
/// Keep the draw bounded if the native creature vector header is corrupt (the mod allows up to 127 enemies).
const MAX_SHIPS: usize = 256;

static ENABLED: AtomicBool = AtomicBool::new(false);
static AI_WAYPOINT: AtomicBool = AtomicBool::new(false);
static PREDICTION_MARKER: AtomicBool = AtomicBool::new(false);
/// Effective ring radii in world units (filtered and capped at `MAX_CIRCLES`); empty = no rings.
static CIRCLE_RADII: Mutex<Vec<f32>> = Mutex::new(Vec::new());

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

/// Store the effective overlay settings; called once from [`crate::ally_reticles::install`].
pub fn configure(config: &crate::DebugOverlayConfig) {
    ENABLED.store(config.enable, Ordering::Relaxed);
    AI_WAYPOINT.store(config.ai_waypoint, Ordering::Relaxed);
    PREDICTION_MARKER.store(config.prediction_marker, Ordering::Relaxed);
    let Ok(mut radii) = CIRCLE_RADII.lock() else {
        return;
    };
    radii.clear();
    radii.extend(
        config
            .circles
            .iter()
            .copied()
            .take(MAX_CIRCLES)
            .filter(|radius| radius.is_finite() && *radius > 0.0),
    );
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

/// Rings around one ship and their world-unit labels. `radii` are the configured world-unit radii
/// (empty = no rings); alpha fades outward by ring index so the set reads as depth.
unsafe fn draw_rings(
    radii: &[f32],
    centre: [f32; 2],
    pixels_per_world: f32,
    rgb: u32,
    font: *mut u8,
) {
    for (index, radius_world) in radii.iter().enumerate() {
        let radius_pixels = radius_world * pixels_per_world;
        if !(MIN_RING_PIXELS..=MAX_RING_PIXELS).contains(&radius_pixels) {
            continue;
        }
        let alpha = (RING_INNER_ALPHA >> index).max(RING_MIN_ALPHA);
        let argb = (alpha << 24) | rgb;
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

/// Smallest positive flight time (seconds) of a straight shell with muzzle speed `speed` against a
/// target `rel` away moving at `relative` velocity (both relative to the shooter), or `None` when
/// the target outruns the shell. Solves `|rel + relative*t| = speed*t` as a quadratic in `t`.
fn intercept_flight_time(rel: [f32; 2], relative: [f32; 2], speed: f32) -> Option<f32> {
    let a = relative[0] * relative[0] + relative[1] * relative[1] - speed * speed;
    let b = 2.0 * (rel[0] * relative[0] + rel[1] * relative[1]);
    let c = rel[0] * rel[0] + rel[1] * rel[1];
    if a.abs() < 1e-4 {
        // shell speed equals the closing rate: flight time degenerates to the linear form
        if b >= 0.0 {
            return None;
        }
        return Some(-c / b);
    }

    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return None;
    }
    let root = discriminant.sqrt();
    let first = (-b - root) / (2.0 * a);
    let second = (-b + root) / (2.0 * a);
    let flight = if first > 0.0 && (second <= 0.0 || first <= second) { first } else { second };
    (flight > 0.0 && flight.is_finite()).then_some(flight)
}

/// The first gun `Body` of the ship's gun groups (group 0 first, then group 1): the same gun the
/// native AI lead reads for its muzzle speed (firing-and-shells.md section 8). `None` when the
/// groups are empty or their headers look corrupt.
unsafe fn first_gun(ship: usize) -> Option<usize> {
    for group in [OFF_GUN_GROUP_0, OFF_GUN_GROUP_1] {
        let begin = rd::<usize>(ship + group);
        let end = rd::<usize>(ship + group + size_of::<usize>());
        if !crate::plausible(begin) || end <= begin || (end - begin) % size_of::<Link>() != 0 {
            continue;
        }
        if (end - begin) / size_of::<Link>() > MAX_GUNS_PER_GROUP {
            continue;
        }
        let gun = rd::<usize>(begin + offset_of!(Link, target));
        if crate::plausible(gun) {
            return Some(gun);
        }
    }
    None
}

/// The loaded ammo's muzzle speed of `gun`: `Body::weapon_ammo` (`+0x850`) is the index into the
/// ammo list (structures.md: `m_weapon_ammo`, 0..35 in vanilla) and `Ammo::speed` (`+0x158`) is the
/// speed the native lead also passes to its intercept solver (firing-and-shells.md section 8).
/// `None` when anything is unreadable instead of guessing a speed.
unsafe fn gun_muzzle_speed(gun: usize) -> Option<f32> {
    let index = rd::<i32>(gun + offset_of!(Body, weapon_ammo));
    if index < 0 {
        return None;
    }

    let ammo_size = size_of::<Ammo>();
    let begin = rd::<usize>(AMMO_LIST);
    let end = rd::<usize>(AMMO_LIST + size_of::<usize>());
    if !crate::plausible(begin) || end <= begin || (end - begin) % ammo_size != 0 {
        return None;
    }
    let count = (end - begin) / ammo_size;
    if count > AMMO_LIST_LIMIT || index as usize >= count {
        return None;
    }

    let record = begin + index as usize * ammo_size;
    let speed = rd::<f32>(record + offset_of!(Ammo, speed));
    (speed.is_finite() && speed > 0.0).then_some(speed)
}

/// What the lead markers are computed from each frame: the player-flown ship's hull centre and
/// its screen position, its root body's velocity, the muzzle speed of its first gun's loaded ammo,
/// and the pixel distance the gun-bearing circles sit from the hull centre at the current zoom.
struct PlayerGuns {
    world: [f32; 2],
    screen: [f32; 2],
    velocity: [f32; 2],
    muzzle_speed: f32,
    bearing_distance_px: f32,
}

/// The player-shooting state the prediction markers need, or `None` when the player ship, its root
/// body or its first gun cannot be read (no guns fitted, for example).
unsafe fn player_guns(player: usize) -> Option<PlayerGuns> {
    let world = [
        rd::<f32>(player + offset_of!(Creature, bounds_center_x)),
        rd::<f32>(player + offset_of!(Creature, bounds_center_y)),
    ];
    let root = crate::bomber::root_part(player)?;
    let velocity = [
        rd::<f32>(root + offset_of!(Body, velocity_x)),
        rd::<f32>(root + offset_of!(Body, velocity_y)),
    ];
    let muzzle_speed = gun_muzzle_speed(first_gun(player)?)?;
    if !world[0].is_finite()
        || !world[1].is_finite()
        || !velocity[0].is_finite()
        || !velocity[1].is_finite()
    {
        return None;
    }

    // the bearing circle needs the hull's screen position and size at this frame's camera zoom
    let mut screen = world;
    let mut reference = [world[0] + SCALE_REFERENCE_WORLD, world[1]];
    project(&mut screen);
    project(&mut reference);
    let mut bearing_distance_px = 0.0;
    if screen[0].is_finite()
        && screen[1].is_finite()
        && reference[0].is_finite()
        && reference[1].is_finite()
    {
        let dx = reference[0] - screen[0];
        let dy = reference[1] - screen[1];
        let pixels_per_world = (dx * dx + dy * dy).sqrt() / SCALE_REFERENCE_WORLD;
        let radius = rd::<f32>(player + offset_of!(Creature, bounding_radius));
        let zoom_ok = pixels_per_world.is_finite() && pixels_per_world > 0.0;
        let radius_ok = radius.is_finite() && radius > 0.0;
        if zoom_ok && radius_ok {
            // just outside the hull's screen radius, so the circle hugs the ship at any zoom
            let hull_pixels = radius * pixels_per_world;
            let distance = hull_pixels + BEARING_MARGIN_PX;
            bearing_distance_px = distance.clamp(BEARING_MIN_PX, BEARING_MAX_PX);
        }
    }

    Some(PlayerGuns { world, screen, velocity, muzzle_speed, bearing_distance_px })
}

/// The lead marker on one hostile ship: where to point the player's first gun to hit it. The
/// shell's world velocity is the shooter's velocity plus the muzzle velocity
/// (firing-and-shells.md section 7), so the marker sits at the target's own future position after
/// the flight time from [`intercept_flight_time`]. Unlike the native AI lead (section 8) this is a
/// player aid: it uses the raw root-body velocity, not the game's smoothed target-velocity ring
/// with its 200-unit speed reduction. Off-screen markers are simply skipped.
unsafe fn draw_prediction(ship: usize, world: [f32; 2], guns: &PlayerGuns, font: *mut u8) {
    let Some(root) = crate::bomber::root_part(ship) else {
        return;
    };
    let velocity = [
        rd::<f32>(root + offset_of!(Body, velocity_x)),
        rd::<f32>(root + offset_of!(Body, velocity_y)),
    ];
    if !velocity[0].is_finite() || !velocity[1].is_finite() {
        return;
    }

    let rel = [world[0] - guns.world[0], world[1] - guns.world[1]];
    let relative = [velocity[0] - guns.velocity[0], velocity[1] - guns.velocity[1]];
    let Some(flight) = intercept_flight_time(rel, relative, guns.muzzle_speed) else {
        return;
    };
    let aim = [world[0] + velocity[0] * flight, world[1] + velocity[1] * flight];

    let mut aim_screen = aim;
    project(&mut aim_screen);
    if !aim_screen[0].is_finite() || !aim_screen[1].is_finite() {
        return;
    }

    // the bearing circle at the ship itself is useful even when the lead point is off-screen
    draw_gun_bearing(aim_screen, guns);

    if !(-100.0..=2020.0).contains(&aim_screen[0]) || !(-100.0..=1180.0).contains(&aim_screen[1]) {
        return;
    }

    draw_ring(aim_screen[0], aim_screen[1], 4.0, PREDICTION_COLOR);
    draw_ring(aim_screen[0], aim_screen[1], 9.0, PREDICTION_HALO_COLOR);
    let mut text = LineBuf::new();
    let _ = write!(text, "AIM {flight:.1}s");
    draw_text(font, aim_screen[0], (aim_screen[1] - 18.0).max(8.0), 2, &text.finish());
}

/// The "put guns here" circle: a small ring just outside the player's own hull, on the screen
/// bearing from the ship to the lead marker. One circle per hostile lead solution, in the same
/// hostile red; skipped when the player's screen position or hull radius could not be read.
unsafe fn draw_gun_bearing(aim_screen: [f32; 2], guns: &PlayerGuns) {
    if guns.bearing_distance_px <= 0.0 {
        return;
    }
    let dx = aim_screen[0] - guns.screen[0];
    let dy = aim_screen[1] - guns.screen[1];
    let distance = dx.hypot(dy);
    if !distance.is_finite() || distance < 1.0 {
        return;
    }

    let scale = guns.bearing_distance_px / distance;
    let x = guns.screen[0] + dx * scale;
    let y = guns.screen[1] + dy * scale;
    draw_ring(x, y, BEARING_RING_PX, PREDICTION_COLOR);
    draw_ring(x, y, BEARING_HALO_PX, PREDICTION_HALO_COLOR);
}

/// Draws rings, state labels, the AI waypoint and (for hostiles) the lead marker for one live
/// ship. `radii` are the configured ring radii (empty = no rings) and `player` the player-shooting
/// state the lead markers need. Returns false when off-screen.
unsafe fn draw_ship(
    ship: usize,
    font: *mut u8,
    radii: &[f32],
    player: Option<&PlayerGuns>,
) -> bool {
    let world = [
        rd::<f32>(ship + offset_of!(Creature, bounds_center_x)),
        rd::<f32>(ship + offset_of!(Creature, bounds_center_y)),
    ];
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
    if !radii.is_empty() {
        draw_rings(radii, centre, pixels_per_world, rgb, font);
    }

    if side < 0.0 {
        if let Some(guns) = player {
            draw_prediction(ship, world, guns, font);
        }
    }

    let role = rd::<i32>(ship + crate::OFF_MODE_D70);
    let label_y = (centre[1] + FONT_OFFSET_Y).max(8.0);
    let next_y = draw_ship_labels(ship, centre, label_y, role, font);
    if AI_WAYPOINT.load(Ordering::Relaxed) {
        draw_waypoint(ship, world, centre, role, next_y, font);
    }
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

    let Ok(radii) = CIRCLE_RADII.lock() else {
        return;
    };

    // collect live ships once; the drawing pass and the player lookup share the same list
    let (begin, end): (usize, usize) = (rd(crate::CREATURES_BEGIN), rd(crate::CREATURES_END));
    if begin != 0 && end > begin {
        let entries = ((end - begin) / 0x18).min(crate::CREATURE_LIST_LIMIT).min(MAX_SHIPS);
        let mut live = [0usize; MAX_SHIPS];
        let mut count = 0;
        for index in 0..entries {
            let ship = rd::<usize>(begin + index * 0x18 + 8);
            if !crate::live_ship(ship) {
                continue;
            }
            live[count] = ship;
            count += 1;
        }

        // the lead markers need the player's own hull, velocity and first-gun ammo speed
        let player = if PREDICTION_MARKER.load(Ordering::Relaxed) {
            live[..count]
                .iter()
                .copied()
                .find(|&ship| rd::<i32>(ship + crate::OFF_MODE_D70) == 1)
                .and_then(|ship| player_guns(ship))
        } else {
            None
        };

        for index in 0..count {
            draw_ship(live[index], font, &radii, player.as_ref());
        }
    }

    style(FONT_NAME.as_ptr(), old_scale, old_blend, old_color);
}
