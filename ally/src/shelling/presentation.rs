//! Spectator raid framing, tracers and native impact effects. See docs/strategic-shelling.md.
//! Presentation takes copied world coordinates, never a ship or a damage packet.

use std::cell::Cell;

use patchy::{Condition, PatchSession, ReturnType, Trampoline};

use crate::{native::read, sound};

use super::visual;

// Gfx_DrawWorldLine and WorldToScreen use this contiguous target/fast/slow-shake block.
const CAMERA: usize = 0x1_4807_5ce8;
const ZOOM: usize = 0x1_403c_11c4;
const PROJECTION_ORIGIN: usize = 0x1_4801_fd10;
const FRAME_WIDTH: usize = 0x1_43a1_58fc;
const FRAME_HEIGHT: usize = 0x1_43a1_3e5c;
const GROUND_HEIGHT_AT: usize = 0x1_402e_e0c0;
const GROUND_POINT_COUNT: usize = 0x1_4800_ee1c;
const SPAWN_EXPLOSION_FX: usize = 0x1_401c_7270;
const MAX_ZOOM: f32 = 1.8;
const FRAME_SIDE_PADDING: f32 = 140.0;
const SKY_HEADROOM: f32 = 200.0;
const GROUND_SCREEN_FRACTION: f32 = 0.93;
const MISS_CLEARANCE: f32 = 95.0;
const MAX_MISS_DROP: f32 = 650.0;
const GROUND_SEARCH_SAMPLES: usize = 9;
const TRACER_ENTRY_MARGIN_PX: f32 = 56.0;
const TRACER_LENGTH_WORLD: f32 = 38.0;

thread_local! {
    // Scope audio suppression to our synchronous FX invocation, not all explosions in a raid.
    static QUIET_IMPACT: Cell<bool> = const { Cell::new(false) };
}

pub(super) type Point = [f32; 2];

pub(super) fn approach_offset(direction: f32) -> Point {
    [direction * 260.0, -320.0]
}

#[derive(Clone, Copy)]
pub(super) struct Hull {
    pub center: Point,
    pub radius: f32,
}

#[derive(Clone, Copy)]
pub(super) struct Frame {
    ground_x: Point,
    ground_y: Point,
}

impl Frame {
    /// Snapshot the opening defender group, including a patch of ground at each flank.
    ///
    /// # Safety
    /// Called from the game thread after the native tactical landscape is loaded.
    pub(super) unsafe fn capture(hulls: impl Iterator<Item = Hull>) -> Option<Self> {
        let mut hulls = hulls;
        let first = hulls.next()?;
        let [x, y] = first.center;
        let mut min = [x - first.radius, y - first.radius];
        let mut max = [x + first.radius, y + first.radius];
        for Hull { center, radius } in hulls {
            for axis in 0..2 {
                min[axis] = min[axis].min(center[axis] - radius);
                max[axis] = max[axis].max(center[axis] + radius);
            }
        }
        let ground_x = [min[0] - MISS_CLEARANCE, max[0] + MISS_CLEARANCE];
        let max_drop = max[1] + MAX_MISS_DROP;
        let mut ground_y = [min[1], max[1]];
        for i in 0..GROUND_SEARCH_SAMPLES {
            let fraction = i as f32 / (GROUND_SEARCH_SAMPLES - 1) as f32;
            let x = ground_x[0] + (ground_x[1] - ground_x[0]) * fraction;
            if let Some([_, y]) = ground_point(x).filter(|&[_, y]| (min[1]..=max_drop).contains(&y))
            {
                ground_y[1] = ground_y[1].max(y);
            }
        }
        Some(Self { ground_x, ground_y })
    }

    /// Sample another piece of actual terrain in the same fixed shot.
    ///
    /// # Safety
    /// Same live game-thread landscape as [`Self::capture`].
    pub(super) unsafe fn miss_point(
        self,
        scatter: f32,
        hulls: impl Iterator<Item = Hull> + Clone,
    ) -> Option<Point> {
        (0..GROUND_SEARCH_SAMPLES)
            .filter_map(|i| {
                let fraction = (scatter + i as f32 / GROUND_SEARCH_SAMPLES as f32).fract();
                ground_point(self.ground_x[0] + (self.ground_x[1] - self.ground_x[0]) * fraction)
            })
            .find(|&point| {
                (self.ground_y[0]..=self.ground_y[1]).contains(&point[1])
                    && clear_of_hulls(point, hulls.clone())
            })
    }

    /// Apply the same camera to the entire battle draw, then put stock state back.
    ///
    /// # Safety
    /// Runs on the native battle renderer, which owns the projection globals for this synchronous
    /// draw (stock inset rendering also temporarily swaps them). The callback may not retain them.
    pub(super) unsafe fn render(self, draw: impl FnOnce()) {
        let width = read::<i32>(FRAME_WIDTH) as f32;
        let height = read::<i32>(FRAME_HEIGHT) as f32;
        let [origin_x, origin_y]: Point = read(PROJECTION_ORIGIN);
        let [left, right] = self.ground_x;
        let [top, ground] = self.ground_y;
        let ground_pixels = height * GROUND_SCREEN_FRACTION;
        let zoom = (width / (right - left + FRAME_SIDE_PADDING * 2.0))
            .min(ground_pixels / (ground - top + SKY_HEADROOM))
            .min(MAX_ZOOM);
        if zoom <= 0.0 || ![zoom, origin_x, origin_y].iter().all(|n| n.is_finite()) {
            draw();
            return;
        }
        let position = [
            (left + right) * 0.5 + (origin_x - width * 0.5) / zoom,
            ground + (origin_y - ground_pixels) / zoom,
        ];
        let camera = CAMERA as *mut [f32; 6];
        let zoom_field = ZOOM as *mut f32;
        // SAFETY: these fixed 1.163 scalar globals belong to the native camera; no pointers are
        // retained and no references to native state are created. Suppress shake only for this draw.
        let previous_camera = camera.read_unaligned();
        let previous_zoom = zoom_field.read_unaligned();
        camera.write_unaligned([position[0], position[1], 0.0, 0.0, 0.0, 0.0]);
        zoom_field.write_unaligned(zoom);
        draw();
        zoom_field.write_unaligned(previous_zoom);
        camera.write_unaligned(previous_camera);
    }
}

pub(super) fn clear_of_hulls([x, y]: Point, mut hulls: impl Iterator<Item = Hull>) -> bool {
    hulls.all(|hull| {
        let [hx, hy] = hull.center;
        (x - hx).hypot(y - hy) > hull.radius + MISS_CLEARANCE * 0.5
    })
}

unsafe fn ground_point(x: f32) -> Option<Point> {
    if !x.is_finite() || read::<i32>(GROUND_POINT_COUNT) <= 0 {
        return None;
    }
    let ground_at: unsafe extern "C" fn(*mut Point, *const Point, *mut i32) -> *mut Point =
        std::mem::transmute(GROUND_HEIGHT_AT);
    let mut ground = [0.0, f32::NAN];
    // SAFETY: the tactical landscape is loaded; the helper only writes to the caller-owned output.
    ground_at(&mut ground, &[x, 0.0], std::ptr::null_mut());
    // A small lift keeps the center of the flash just above the terrain.
    ground[1].is_finite().then_some([x, ground[1] - 2.0])
}

/// Emit stock flash, smoke and flying debris without dispatching Body_Explode or its damage pass.
///
/// # Safety
/// Game-thread raid tick with the native world and effects manager initialized. `point` is a
/// finite world coordinate. The stock effect routine owns all objects it creates.
pub(super) unsafe fn impact(point: Point, ground: bool, intensity: f32, cue: u32) {
    // ABI from World_ThermalPass at 0x140128aee. The +0x10 request field controls
    // visual size; the last float is the light's z. Kind 0 is an ordinary burst.
    let spawn: unsafe extern "C" fn(*const Point, f32, i32, f32, f32, f32, u8, i32, f32) =
        std::mem::transmute(SPAWN_EXPLOSION_FX);
    let variation = (cue & 0xff) as f32 / 255.0;
    let size = if ground {
        13.0 + 5.0 * variation + 5.0 * intensity
    } else {
        15.0 + 4.0 * variation
    };
    let _ = QUIET_IMPACT.try_with(|quiet| {
        let previous = quiet.replace(true);
        spawn(&point, 0.0, 0, size, 0.0, 0.0, 0, 41, 1.0);
        quiet.set(previous);
    });
    sound::shelling_impact(ground, intensity, cue);
}

extern "C" fn use_quiet_impact() -> u8 {
    u8::from(QUIET_IMPACT.try_with(Cell::get).unwrap_or(false))
}

pub(super) unsafe fn install(session: &mut PatchSession) -> Result<(), String> {
    // FX_SpawnExplosionDebris's sound-only tail. The stock quiet branch already targets
    // 0x1401c9703, which starts cleanup. Preserve the stock registers on both paths.
    let mut audio = Trampoline::new();
    let cleanup = audio.new_label();
    audio.preserved_call(
        use_quiet_impact as *const (),
        &[],
        &[0x84, 0xc0], // test al, al
        ReturnType::None,
    );
    audio.jump_if(Condition::NotEqual, cleanup);
    audio.bytes(&[0x44, 0x0f, 0x2f, 0xc8]); // comiss xmm9, xmm0
    audio.jump_if(Condition::Above, cleanup);
    audio.absolute_jump(0x1_401c_9389);
    audio.bind(cleanup).map_err(|error| error.to_string())?;
    audio.absolute_jump(0x1_401c_9703);
    session
        .detour_trampoline(0x1_401c_937f, 10, audio)
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) struct SkyEntry {
    above_top: f32,
}

impl SkyEntry {
    /// Invert the tactical world-line projection so the shot starts beyond the actual viewport,
    /// including on wider shots that zoom out to fit the whole garrison.
    ///
    /// # Safety
    /// Called on the battle render seam with its native projection installed.
    pub(super) unsafe fn capture() -> Option<Self> {
        let [_, y, _, fast_y, _, slow_y]: [f32; 6] = read(CAMERA);
        let [_, origin_y]: Point = read(PROJECTION_ORIGIN);
        let zoom: f32 = read(ZOOM);
        if !zoom.is_finite() || zoom <= 0.0 {
            return None;
        }
        let top = y + fast_y + slow_y - origin_y / zoom;
        let above_top = top - TRACER_ENTRY_MARGIN_PX / zoom;
        above_top.is_finite().then_some(Self { above_top })
    }

    /// Draw a descending shell along the exact straight-line angle of the damage sightline.
    ///
    /// # Safety
    /// The battle renderer still owns the projection captured by [`Self::capture`].
    pub(super) unsafe fn tracer(&self, target: Point, direction: f32, fraction: f32) {
        let left = 1.0 - fraction.clamp(0.0, 1.0);
        let [dx, dy] = approach_offset(direction);
        let rise = (target[1] - self.above_top).max(-dy);
        let drift = dx * (rise / -dy);
        let head = [target[0] + drift * left, target[1] - rise * left];
        let tail_scale = TRACER_LENGTH_WORLD / drift.hypot(rise);
        let tail = [head[0] + drift * tail_scale, head[1] - rise * tail_scale];
        // Vanilla 180 mm HE shells (FireShell case 0x19) use lightPower 10000 with the stock
        // warm colour (1.0, 0.9843, 0.8235) and a fast light fade. Layer the same palette as
        // wide translucent quads over a bright core; no native projectile is spawned.
        visual::world_line(tail, head, 14.0, 0x1a_ff_c9_69);
        visual::world_line(tail, head, 8.0, 0x46_ff_d9_88);
        visual::world_line(tail, head, 4.5, 0xc8_ff_f4_cd);
        visual::world_line(tail, head, 2.0, 0xffff_ffff);
    }
}
