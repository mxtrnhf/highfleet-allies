//! Per-round strategic shell flight and impact rendering.
//!
//! Every round has a launch time, a flight time and a deterministic effective-hit result. The
//! parent consumes effective impacts as they land; rendering and damage therefore share one
//! simulation rather than rolling unrelated damage when a countdown expires.
//!
//! Drawing uses the game's native `Gfx_DrawWorldLine` (`0x140248ba0`) in world-coordinate mode.
//! The earlier font-marker overlay submitted no visible geometry on the campaign map. Native world
//! lines perform the game's own campaign projection and clipping and need no font or sprite state.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        Mutex,
    },
    time::{Duration, Instant},
};

use super::estimate::shell_hit;

/// Upper bound on individually rendered shells in one frame. All rounds remain simulated; at
/// extreme cadence the drawn rounds are sampled across the entire route instead of at the muzzle.
pub const MAX_SHELLS: usize = 24;
/// Upper bound on impact bursts drawn in one frame.
pub const MAX_IMPACTS: usize = 24;
/// Bound schedule construction even with badly modified weapon stats.
const MAX_SIMULATED_ROUNDS: u32 = 250_000;

const DRAW_WORLD_LINE: usize = 0x1_4024_8ba0;
const FLIGHT_FRACTION: f64 = 0.55;
const MIN_FLIGHT_SECONDS: f64 = 120.0;
const MAX_FLIGHT_SECONDS: f64 = 1_800.0;
const BURST_FRACTION: f64 = 0.06;
const MIN_BURST_SECONDS: f64 = 15.0;
/// Give the last bursts a visible fade even when the final campaign tick jumps.
pub(super) const BURST_TAIL: Duration = Duration::from_millis(650);
const SHELL_LENGTH_WORLD: f32 = 2.8;
const SHELL_CROSS_WORLD: f32 = 1.8;
const SHELL_COLOR: u32 = 0xffff_f2a0;

pub(super) fn flight_seconds(duration_seconds: f64) -> f64 {
    (duration_seconds * FLIGHT_FRACTION)
        .clamp(MIN_FLIGHT_SECONDS, MAX_FLIGHT_SECONDS)
        .min(duration_seconds * 0.8)
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeVec2 {
    x: f32,
    y: f32,
}

impl From<[f32; 2]> for NativeVec2 {
    fn from([x, y]: [f32; 2]) -> Self {
        Self { x, y }
    }
}

type DrawWorldLineFn =
    unsafe extern "win64" fn(NativeVec2, NativeVec2, f32, u32, u32, u8, f32, u32, f32);

/// The 180 mm family firing the barrage, inferred from sustained rounds per mount per minute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cannon {
    Single180,
    Sarmat180,
    Squall180,
}

impl Cannon {
    pub fn from_rounds_per_mount_per_minute(rate: f64) -> Self {
        if rate <= 14.0 {
            Self::Single180
        } else if rate <= 30.0 {
            Self::Sarmat180
        } else {
            Self::Squall180
        }
    }

    fn burst_scale(self) -> f32 {
        match self {
            Self::Single180 => 1.0,
            Self::Sarmat180 => 1.2,
            Self::Squall180 => 1.5,
        }
    }
}

/// Borrowed-by-value battery summary. No game pointers are retained.
#[derive(Clone, Copy, Debug)]
pub struct Composition {
    pub mounts: u32,
    pub rounds: u32,
    pub magazine_weight: u32,
    pub cannon: Cannon,
}

impl Composition {
    pub fn new(mounts: u32, rounds: u32, magazine_weight: u32, duration_seconds: f64) -> Self {
        let cannon = if mounts == 0 || !duration_seconds.is_finite() || duration_seconds <= 0.0 {
            Cannon::Single180
        } else {
            let minutes = duration_seconds / 60.0;
            let rate = f64::from(rounds) / f64::from(mounts) / minutes;
            Cannon::from_rounds_per_mount_per_minute(rate)
        };
        Self {
            mounts,
            rounds,
            magazine_weight,
            cannon,
        }
    }
}

/// Immutable per-round simulation for one barrage.
#[derive(Clone, Debug)]
pub struct Plan {
    origin: [f32; 2],
    target: [f32; 2],
    start_clock: f64,
    duration_seconds: f64,
    flight_seconds: f64,
    launch_window: f64,
    period_seconds: f64,
    rounds: u32,
    burst_lifetime: f64,
    cannon: Cannon,
    mounts: u32,
    magazine_weight: u32,
    effective_hits: Vec<u32>,
}

impl Plan {
    pub fn new(
        origin: [f32; 2],
        target: [f32; 2],
        start_clock: f64,
        duration_seconds: f64,
        composition: Composition,
        round_hit_chance: f64,
        seed: u64,
    ) -> Option<Self> {
        let finite = origin
            .iter()
            .chain(target.iter())
            .all(|value| value.is_finite());
        if !finite
            || !start_clock.is_finite()
            || !duration_seconds.is_finite()
            || duration_seconds <= 0.0
            || composition.rounds == 0
            || composition.rounds > MAX_SIMULATED_ROUNDS
        {
            return None;
        }
        let flight_seconds = flight_seconds(duration_seconds);
        let launch_window = (duration_seconds - flight_seconds).max(1.0);
        let rounds = composition.rounds;
        let period_seconds = launch_window / f64::from(rounds);
        let effective_hits = (0..rounds)
            .filter(|&index| shell_hit(seed, index, round_hit_chance))
            .collect();
        Some(Self {
            origin,
            target,
            start_clock,
            duration_seconds,
            flight_seconds,
            launch_window,
            period_seconds,
            rounds,
            burst_lifetime: (duration_seconds * BURST_FRACTION).max(MIN_BURST_SECONDS),
            cannon: composition.cannon,
            mounts: composition.mounts,
            magazine_weight: composition.magazine_weight,
            effective_hits,
        })
    }

    pub fn elapsed(&self, clock: f64) -> Option<f64> {
        if !clock.is_finite() || clock < self.start_clock {
            return None;
        }
        Some((clock - self.start_clock) * 3_600.0)
    }

    pub fn active_at(&self, elapsed: f64) -> bool {
        elapsed >= 0.0
            && elapsed <= self.duration_seconds + self.burst_lifetime
            && self.launch_window > 0.0
            && self.period_seconds > 0.0
            && self.flight_seconds > 0.0
    }

    fn launch_time(&self, index: u32) -> f64 {
        f64::from(index) * self.period_seconds
    }

    fn is_effective_hit(&self, index: u32) -> bool {
        self.effective_hits.binary_search(&index).is_ok()
    }

    /// Number of effective shells whose individual flight has reached the city.
    fn landed_hit_count(&self, elapsed: f64) -> u32 {
        let latest = ((elapsed - self.flight_seconds) / self.period_seconds).floor() as i64;
        if latest < 0 {
            return 0;
        }
        let latest = (latest as u64).min(u64::from(self.rounds.saturating_sub(1))) as u32;
        self.effective_hits
            .partition_point(|&index| index <= latest) as u32
    }

    fn shell_spread(index: u32) -> f32 {
        ((index.wrapping_mul(37) % 17) as f32 - 8.0) * 2.0
    }

    fn impact_scatter(&self, index: u32) -> [f32; 2] {
        let scale = 1.0 + self.magazine_weight.min(96) as f32 * 0.006;
        [
            ((index.wrapping_mul(29) % 13) as f32 - 6.0) * 2.5 * scale,
            ((index.wrapping_mul(53) % 11) as f32 - 5.0) * 2.5 * scale,
        ]
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ShellPosition {
    pub world: [f32; 2],
    pub progress: f32,
    pub rotation: f32,
    pub spread: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Impact {
    pub world: [f32; 2],
    pub age_fraction: f32,
    pub scatter: [f32; 2],
    pub effective: bool,
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub shells: [ShellPosition; MAX_SHELLS],
    pub shell_count: usize,
    pub impacts: [Impact; MAX_IMPACTS],
    pub impact_count: usize,
    pub cannon: Cannon,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            shells: [ShellPosition::default(); MAX_SHELLS],
            shell_count: 0,
            impacts: [Impact::default(); MAX_IMPACTS],
            impact_count: 0,
            cannon: Cannon::Single180,
        }
    }
}

/// Derives visible rounds and bursts from the per-round simulation without native calls.
pub fn sample_frame(plan: &Plan, elapsed: f64) -> Frame {
    let mut frame = Frame::default();
    if !plan.active_at(elapsed) {
        return frame;
    }
    frame.cannon = plan.cannon;
    let direction = [
        plan.target[0] - plan.origin[0],
        plan.target[1] - plan.origin[1],
    ];
    let rotation = direction[1].atan2(direction[0]);
    let last_index = ((elapsed / plan.period_seconds).floor() as u64)
        .min(u64::from(plan.rounds.saturating_sub(1)));
    let first_index =
        (((elapsed - plan.flight_seconds) / plan.period_seconds).floor() as i64 + 1).max(0) as u64;
    if first_index <= last_index {
        // Pin the visible subset to whole-barrage indices. A sliding stride/phase
        // swaps dots out mid-flight and looks like sparkles instead of movement.
        let peak_airborne = (plan.flight_seconds / plan.period_seconds).ceil() as u64;
        let stride = peak_airborne
            .min(u64::from(plan.rounds))
            .max(1)
            .div_ceil(MAX_SHELLS as u64);
        let first_visible = first_index.div_ceil(stride) * stride;
        for index in (first_visible..=last_index).step_by(stride as usize) {
            let since_launch = elapsed - plan.launch_time(index as u32);
            if !(0.0..plan.flight_seconds).contains(&since_launch)
                || frame.shell_count >= MAX_SHELLS
            {
                continue;
            }
            let fraction = (since_launch / plan.flight_seconds) as f32;
            let [scatter_x, scatter_y] = plan.impact_scatter(index as u32);
            frame.shells[frame.shell_count] = ShellPosition {
                world: [
                    plan.origin[0] + (direction[0] + scatter_x) * fraction,
                    plan.origin[1] + (direction[1] + scatter_y) * fraction,
                ],
                progress: fraction,
                rotation,
                spread: Plan::shell_spread(index as u32)
                    * (1.0 + plan.mounts.min(32) as f32 * 0.02),
            };
            frame.shell_count += 1;
        }
    }

    let landed_latest = ((elapsed - plan.flight_seconds) / plan.period_seconds).floor() as i64;
    if landed_latest >= 0 {
        let oldest = (((elapsed - plan.flight_seconds - plan.burst_lifetime) / plan.period_seconds)
            .floor() as i64)
            .max(0);
        let newest = (landed_latest as u64).min(u64::from(plan.rounds.saturating_sub(1)));
        for step in 0..newest.saturating_sub(oldest as u64) + 1 {
            if frame.impact_count >= MAX_IMPACTS {
                break;
            }
            let index = newest - step;
            let age = elapsed - plan.launch_time(index as u32) - plan.flight_seconds;
            if !(0.0..=plan.burst_lifetime).contains(&age) {
                continue;
            }
            frame.impacts[frame.impact_count] = Impact {
                world: plan.target,
                age_fraction: (age / plan.burst_lifetime) as f32,
                scatter: plan.impact_scatter(index as u32),
                effective: plan.is_effective_hit(index as u32),
            };
            frame.impact_count += 1;
        }
    }
    frame
}

struct State {
    plan: Option<Plan>,
    elapsed: f64,
    finished_at: Option<Instant>,
}

impl State {
    const fn new() -> Self {
        Self {
            plan: None,
            elapsed: 0.0,
            finished_at: None,
        }
    }
}

static STATE: Mutex<State> = Mutex::new(State::new());
static DRAW_LOGGED: AtomicBool = AtomicBool::new(false);

pub(crate) fn begin(
    origin: [f32; 2],
    target: [f32; 2],
    start_clock: f64,
    duration_seconds: f64,
    composition: Composition,
    round_hit_chance: f64,
    seed: u64,
) -> bool {
    let Some(plan) = Plan::new(
        origin,
        target,
        start_clock,
        duration_seconds,
        composition,
        round_hit_chance,
        seed,
    ) else {
        return false;
    };
    if let Ok(mut state) = STATE.try_lock() {
        state.plan = Some(plan);
        state.elapsed = 0.0;
        state.finished_at = None;
        DRAW_LOGGED.store(false, Relaxed);
        true
    } else {
        false
    }
}

pub(crate) fn clear() {
    if let Ok(mut state) = STATE.try_lock() {
        state.plan = None;
        state.elapsed = 0.0;
        state.finished_at = None;
    }
}

/// Effective shells that have landed by this campaign clock, including hits from earlier frames.
pub(crate) fn landed_hit_count(clock: f64) -> Option<u32> {
    let state = STATE.try_lock().ok()?;
    let plan = state.plan.as_ref()?;
    Some(plan.landed_hit_count(plan.elapsed(clock)?))
}

fn sample(clock: f64) -> bool {
    let Ok(mut state) = STATE.try_lock() else {
        return false;
    };
    let Some(plan) = state.plan.as_ref() else {
        return false;
    };
    let Some(elapsed) = plan.elapsed(clock) else {
        return false;
    };
    let (duration, lifetime) = (plan.duration_seconds, plan.burst_lifetime);
    if elapsed < duration {
        state.elapsed = elapsed;
        return true;
    }
    let now = Instant::now();
    let finished = *state.finished_at.get_or_insert(now);
    let tail = now.duration_since(finished).as_secs_f64() / BURST_TAIL.as_secs_f64();
    state.elapsed = duration + tail * lifetime;
    tail < 1.0
}

/// Submits one native world-space line. Width and colour are screen-space HGE quad properties.
pub(super) unsafe fn world_line(from: [f32; 2], to: [f32; 2], width: f32, color: u32) {
    let draw: DrawWorldLineFn = std::mem::transmute(DRAW_WORLD_LINE);
    draw(from.into(), to.into(), width, color, 0, 0, 0.0, 1, 0.0);
}

unsafe fn draw_shells(frame: &Frame) {
    for shell in frame.shells.iter().take(frame.shell_count) {
        let direction = [shell.rotation.cos(), shell.rotation.sin()];
        let side = [-direction[1], direction[0]];
        let curvature = (std::f32::consts::PI * shell.progress).sin();
        let spread = shell.spread * 0.35 * curvature;
        let arc = curvature * 1.5;
        let center = [
            shell.world[0] + side[0] * spread,
            shell.world[1] + side[1] * spread - arc,
        ];
        world_line(
            [
                center[0] - direction[0] * SHELL_LENGTH_WORLD,
                center[1] - direction[1] * SHELL_LENGTH_WORLD,
            ],
            [
                center[0] + direction[0] * SHELL_LENGTH_WORLD,
                center[1] + direction[1] * SHELL_LENGTH_WORLD,
            ],
            4.5,
            SHELL_COLOR,
        );
        world_line(
            [
                center[0] - side[0] * SHELL_CROSS_WORLD,
                center[1] - side[1] * SHELL_CROSS_WORLD,
            ],
            [
                center[0] + side[0] * SHELL_CROSS_WORLD,
                center[1] + side[1] * SHELL_CROSS_WORLD,
            ],
            2.5,
            SHELL_COLOR,
        );
    }
}

unsafe fn draw_impacts(frame: &Frame) {
    for impact in frame.impacts.iter().take(frame.impact_count) {
        let center = [
            impact.world[0] + impact.scatter[0],
            impact.world[1] + impact.scatter[1],
        ];
        let fade = (1.0 - impact.age_fraction).clamp(0.2, 1.0);
        let alpha = (fade * if impact.effective { 255.0 } else { 120.0 }) as u32;
        let color = (alpha << 24)
            | if impact.effective {
                0x00ff_7838
            } else {
                0x00ff_d0a0
            };
        let radius = frame.cannon.burst_scale()
            * if impact.effective { 5.0 } else { 2.5 }
            * (0.8 + impact.age_fraction.clamp(0.0, 1.0));
        world_line(
            [center[0] - radius, center[1]],
            [center[0] + radius, center[1]],
            if impact.effective { 4.0 } else { 2.0 },
            color,
        );
        world_line(
            [center[0], center[1] - radius],
            [center[0], center[1] + radius],
            if impact.effective { 4.0 } else { 2.0 },
            color,
        );
    }
}

/// Draws the current per-round simulation at the parent's native map-render seam.
unsafe fn draw() {
    let (frame, rounds, effective_hits, origin, target) = {
        let Ok(state) = STATE.try_lock() else {
            return;
        };
        let Some(plan) = state.plan.as_ref() else {
            return;
        };
        if !plan.active_at(state.elapsed) {
            return;
        }
        (
            sample_frame(plan, state.elapsed),
            plan.rounds,
            plan.effective_hits.len(),
            plan.origin,
            plan.target,
        )
    };
    if !DRAW_LOGGED.swap(true, Relaxed) {
        log::info!(
            "hf_ally: shelling native-line frame: {} airborne, {} bursts, {rounds} simulated rounds, {effective_hits} effective hits, origin={origin:?}, target={target:?}",
            frame.shell_count,
            frame.impact_count,
        );
    }
    draw_shells(&frame);
    draw_impacts(&frame);
}

pub(crate) unsafe fn draw_with_clock(clock: f64) {
    if sample(clock) {
        draw();
    } else if let Ok(mut state) = STATE.try_lock() {
        state.elapsed = 0.0;
    }
}
