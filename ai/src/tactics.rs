//! The `tactics` stage ("strafe in the band"): game-memory glue around the pure `tactics_math`. Runs for every AI
//! ship on every AI tick, so the default-off path is one atomic load, and everything read from the game is
//! page-checked (`crate::rd`). Per-ship numbers that need several reads (thrust, mass, reach) are refreshed once a second.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::tactics_config::TacticsConfig;
use crate::tactics_math::{self as tm, Bounds, Gate, GateInput, Params, Rng, ShipState, State, StepInput};
use crate::{plausible, rd, Ctx};

const OFF_AI_FLAG: usize = 0xbd4; // u8: 0 = flown by a player
const OFF_MODE: usize = 0xb48; // i32: 1 airborne combat, 3 landed, 0 takeoff (telemetry column `mode`; NOT +0xd70, that is the role)
/// Fire byte of main-gun group 0 (the player's left mouse button). The AI weapon controller never uses it; CreatureUpdate
/// calls FireGuns for every gun of the group while it is set (each gun obeys its own reload).
const OFF_FIRE0: usize = 0xbbc;
const OFF_ACTIVE: usize = 0x48; // i32
const OFF_STATE: usize = 0xbdc; // i32 driver state, 6 / 7 = dead or gone
const OFF_CODE: usize = 0x28;
const OFF_REACH: usize = 0x1838; // f32 gun reach, +0x183c second fire group (read as a pair)
/// Arena extents the stock AI's map-edge push and the deploy code use: x max, y max, x min, y min (f32 each).
const BOUNDS_ADDRESS: usize = 0x1_403c_2bc0;

const REFRESH_S: f64 = 1.0;
const MAX_ENTRIES: usize = 256;
const MAX_GATE_LOGS: u32 = 24;

static ENABLED: AtomicBool = AtomicBool::new(false);
static PARAMS: OnceLock<Params> = OnceLock::new();
static GATE_LOGS: AtomicU32 = AtomicU32::new(0);
/// `dive_boost` as f32 bits (1.0 = none).
static DIVE_BOOST: AtomicU32 = AtomicU32::new(0x3f80_0000);
static AIM_BOOST: AtomicU32 = AtomicU32::new(0x3f80_0000);
/// Ships whose dodge boost is valid until the given game time. Written by `decide`, read by the engine-command hook.
static BOOSTED: Mutex<Vec<(usize, f64, f32)>> = Mutex::new(Vec::new());
/// The player's aim: two adjacent f32, a world-aligned direction (y not flipped), magnitude up to 100 (measured: points
/// at the target within ~3 deg while firing, see hf_notes findings/ai-telemetry.md).
const AIM_ADDRESS: usize = 0x1_403c_0dc8;
/// Aim vectors shorter than this are not trusted (no aim).
const MIN_AIM_LEN: f32 = 5.0;
/// How long one `dive` decision keeps the boost on (the engine commands go out right after the steering in the same tick).
const BOOST_HOLD_S: f64 = 0.3;

/// Boost factor the engine-command hook applies to `creature` at `now` because of a dodge (1.0 = none).
pub fn dive_boost_for(creature: usize, now: f64) -> f32 {
    let Ok(list) = BOOSTED.try_lock() else { return 1.0 };
    list.iter().filter(|&&(c, until, _)| c == creature && now <= until).map(|&(_, _, factor)| factor).fold(1.0, f32::max)
}

/// Whether the engine hook must be installed for the dodge boost.
pub fn wants_dive_boost() -> bool {
    ENABLED.load(Ordering::Relaxed) && f32::from_bits(DIVE_BOOST.load(Ordering::Relaxed)).max(f32::from_bits(AIM_BOOST.load(Ordering::Relaxed))) > 1.0
}

/// Keeps a waypoint inside the arena (50 u margin at the sides and the ceiling) using the bounds the tactics stage
/// read; unchanged when they are not known (tactics off or no battle yet).
pub fn clamp_to_arena(waypoint: (f32, f32), ship: (f32, f32)) -> (f32, f32) {
    let Ok(guard) = WORLD.try_lock() else { return waypoint };
    let Some(bounds) = guard.as_ref().and_then(|w| w.bounds) else { return waypoint };
    tm::clamp_waypoint(waypoint, ship, &bounds)
}

fn note_boost(creature: usize, now: f64, factor: f32) {
    let Ok(mut list) = BOOSTED.try_lock() else { return };
    list.retain(|&(c, until, _)| c != creature && until >= now - 5.0 && until <= now + 5.0);
    if factor > 1.0 {
        list.push((creature, now + BOOST_HOLD_S, factor));
    }
}

struct Entry {
    st: ShipState,
    rng: Rng,
    next_refresh: f64,
    boosting: bool,
    /// Early fire: window number already rolled for, when the fire ends, and whether we set the fire byte.
    fire_rolled: u32,
    fire_until: f64,
    fire_set: bool,
}


#[derive(Default)]
struct World {
    last_now: f64,
    last_prune: f64,
    ships: HashMap<usize, Entry>,
    bounds: Option<Bounds>,
    next_bounds_read: f64,
    bounds_logged: bool,
    aim_clock: tm::AimClock,
    rng: Option<Rng>,
}

static WORLD: Mutex<Option<World>> = Mutex::new(None);

/// Called once from `init` with the (unsanitized) config block.
pub fn init(config: &TacticsConfig) {
    let params = config.params();
    let _ = PARAMS.set(params);
    ENABLED.store(params.enabled, Ordering::Relaxed);
    let boost = if params.dive_enabled { params.dive_boost } else { 1.0 };
    DIVE_BOOST.store(boost.to_bits(), Ordering::Relaxed);
    AIM_BOOST.store((if params.aim_enabled { params.aim_boost } else { 1.0 }).to_bits(), Ordering::Relaxed);
    if params.enabled {
        log::info!(
            "hf_ai: tactics ON (strafe {}, band k {} in {}..{} +-{}, blend {}, strafe {}..{} s, allies {}; dive {} chance {} cone {} deg range {} dwell {} s boost {})",
            params.strafe_enabled,
            params.k_center,
            params.band_min,
            params.band_max,
            params.half_width,
            params.blend_weight,
            params.strafe_min_s,
            params.strafe_max_s,
            params.apply_to_allies,
            params.dive_enabled,
            params.dive_chance,
            params.dive_cone_deg,
            params.dive_range,
            params.dive_dwell_s,
            params.dive_boost
        );
        if params.aim_enabled {
            log::info!(
                "hf_ai: aim awareness ON (window {} s every {}..{} s, tolerance {} u, range {} u, weight {}, boost {})",
                params.aim_window_s,
                params.aim_gap_min_s,
                params.aim_gap_max_s,
                params.aim_tolerance,
                params.aim_range,
                params.aim_weight,
                params.aim_boost
            );
            if params.aim_fire_probability > 0.0 {
                log::info!(
                    "hf_ai: early fire ON (chance {} per window, {} s, within {} x reach)",
                    params.aim_fire_probability,
                    params.aim_fire_s,
                    params.aim_fire_range
                );
            }
        }
    } else {
        log::info!("hf_ai: tactics off (config tactics.enabled = false)");
    }
}

fn live_ship(p: usize) -> bool {
    // SAFETY: page-checked reads.
    unsafe {
        plausible(p)
            && rd::<u8>(p + OFF_CODE).is_some_and(|c| c & 0x2f == 0x2f)
            && rd::<i32>(p + OFF_ACTIVE).is_some_and(|a| a != 0)
            && rd::<i32>(p + OFF_STATE).is_some_and(|s| !matches!(s, 6 | 7))
    }
}

fn read_reach(creature: usize) -> Option<f32> {
    // SAFETY: page-checked read of two adjacent f32.
    let pair = unsafe { rd::<[f32; 2]>(creature + OFF_REACH) }?;
    tm::best_reach(pair[0], pair[1])
}

/// The tactics decision for one ship. Returns the waypoint (or `None` = leave the stock one) and the state for the log.
pub fn decide(ctx: &Ctx, panicking: impl FnOnce() -> bool) -> (Option<(f32, f32)>, State) {
    let result = decide_inner(ctx, panicking);
    if ANY_FIRE.load(Ordering::Relaxed) {
        release_fire(ctx.creature, ctx.now);
    }
    result
}

/// True while some ship may have its fire byte set by us (so the cleanup after every decision is skipped otherwise).
static ANY_FIRE: AtomicBool = AtomicBool::new(false);

/// Clears the fire byte of `creature` when its early fire is over, whichever early return `decide_inner` took
/// (a panic, a failed gate, a lost lock ...), so a ship never keeps shooting after its window.
fn release_fire(creature: usize, now: f64) {
    let Ok(mut guard) = WORLD.try_lock() else { return };
    let Some(world) = guard.as_mut() else { return };
    if let Some(entry) = world.ships.get_mut(&creature) {
        if entry.fire_set && now >= entry.fire_until {
            // SAFETY: page-checked read, then a one-byte write into the live creature.
            unsafe {
                if rd::<u8>(creature + OFF_FIRE0).is_some() {
                    std::ptr::write_unaligned((creature + OFF_FIRE0) as *mut u8, 0);
                }
            }
            entry.fire_set = false;
        }
    }
    if !world.ships.values().any(|e| e.fire_set) {
        ANY_FIRE.store(false, Ordering::Relaxed);
    }
}

fn decide_inner(ctx: &Ctx, panicking: impl FnOnce() -> bool) -> (Option<(f32, f32)>, State) {
    if !ENABLED.load(Ordering::Relaxed) || !crate::maneuvers_allowed(ctx.creature) {
        return (None, State::Stock);
    }
    let Some(p) = PARAMS.get() else { return (None, State::Stock) };
    // cheap gates first: side, then the AI flag and mode (two reads), then the target
    if !ctx.side.is_finite() || ctx.side == 0.0 || (ctx.side > 0.0 && !p.apply_to_allies) {
        return (None, State::Stock);
    }
    // SAFETY: page-checked reads.
    let (ai_flag, mode) = unsafe { (rd::<u8>(ctx.creature + OFF_AI_FLAG), rd::<i32>(ctx.creature + OFF_MODE)) };
    let (Some(ai_flag), Some(target), Some(target_creature)) = (ai_flag, ctx.target, ctx.target_creature) else {
        return (None, State::Stock);
    };
    if ai_flag == 0 || mode != Some(1) || target_creature == ctx.creature || !live_ship(target_creature) {
        return (None, State::Stock);
    }
    if !(ctx.ship.0.is_finite() && ctx.ship.1.is_finite() && target.0.is_finite() && target.1.is_finite()) {
        return (None, State::Stock);
    }
    // The panic query takes the ai_flares lock; it is called before ours and the two are never held together.
    let is_panicking = panicking();
    if is_panicking {
        return (None, State::Panic);
    }
    let mut guard = match WORLD.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return (None, State::Stock),
    };
    let world = guard.get_or_insert_with(World::default);
    let now = ctx.now;
    if tm::clock_reset(world.last_now, now) {
        world.ships.clear();
        world.bounds = None;
        world.next_bounds_read = 0.0;
        world.bounds_logged = false;
        world.aim_clock = tm::AimClock::default();
        world.last_prune = now;
    }
    world.last_now = now;
    if now - world.last_prune > tm::STALE_AFTER {
        world.last_prune = now;
        world.ships.retain(|_, e| now - e.st.last_seen <= tm::STALE_AFTER);
    }
    if now >= world.next_bounds_read {
        // re-read every second: a new arena may come without a clock reset
        world.next_bounds_read = now + REFRESH_S;
        // SAFETY: page-checked read of four adjacent f32 in the game image.
        let fresh = unsafe { rd::<[f32; 4]>(BOUNDS_ADDRESS) }.and_then(Bounds::from_raw);
        let changed = fresh != world.bounds;
        world.bounds = fresh;
        if changed || !world.bounds_logged {
            world.bounds_logged = true;
            log::info!("hf_ai: tactics arena bounds {:?} (raw read, None = unusable, tactics stay stock)", world.bounds);
        }
    }
    let Some(bounds) = world.bounds else { return (None, State::Stock) };

    if !world.ships.contains_key(&ctx.creature) {
        if world.ships.len() >= MAX_ENTRIES {
            return (None, State::Stock);
        }
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1)
            ^ (ctx.creature as u64).rotate_left(17)
            ^ now.to_bits();
        let mut rng = Rng::new(seed);
        let st = ShipState::new(now, &mut rng, p);
        world.ships.insert(ctx.creature, Entry { st, rng, next_refresh: f64::NEG_INFINITY, boosting: false, fire_rolled: 0, fire_until: 0.0, fire_set: false });
    }
    // Aim awareness: one shared window clock; only ships that target the player see the aim.
    let mut aim = None;
    if p.aim_enabled {
        let seed = now.to_bits() ^ 0x9e37_79b9_7f4a_7c15;
        let rng = world.rng.get_or_insert_with(|| Rng::new(seed));
        if world.aim_clock.update(now, p, rng) {
            // SAFETY: page-checked reads. The target is the player when its AI flag is 0.
            let (target_ai, raw, shooting) = unsafe {
                (
                    rd::<u8>(target_creature + OFF_AI_FLAG),
                    rd::<[f32; 2]>(AIM_ADDRESS),
                    rd::<[u8; 2]>(target_creature + OFF_FIRE0).is_some_and(|f| f[0] != 0 || f[1] != 0),
                )
            };
            if !p.aim_require_fire || shooting {
            if let (Some(0), Some([ax, ay])) = (target_ai, raw) {
                if ax.is_finite() && ay.is_finite() && (ax * ax + ay * ay).sqrt() >= MIN_AIM_LEN {
                    if let Some(dir) = tm::normalized(ax, ay) {
                        aim = Some(tm::AimInput { player: target, dir });
                    }
                }
            }
            }
        }
    }
    let world_aim_window = world.aim_clock.window;
    let Some(entry) = world.ships.get_mut(&ctx.creature) else { return (None, State::Stock) };
    if now >= entry.next_refresh {
        let first = entry.next_refresh == f64::NEG_INFINITY;
        entry.next_refresh = now + REFRESH_S;
        let own = read_reach(ctx.creature);
        let theirs = read_reach(target_creature);
        entry.st.latch_reach(own, target_creature, theirs);
        if first && GATE_LOGS.fetch_add(1, Ordering::Relaxed) < MAX_GATE_LOGS {
            log::info!(
                "hf_ai: tactics ship {:#x}: reach {:?}, target reach {:?}",
                ctx.creature,
                own,
                theirs
            );
        }
    }
    let gate = tm::gate(
        p,
        &GateInput { ai_flag, side: ctx.side, mode, has_target: true, panicking: false },
    );
    if gate != Gate::Pass {
        return (None, State::Stock);
    }
    let step = tm::step(
        p,
        &mut entry.st,
        &bounds,
        &StepInput { now, aim, ship: ctx.ship, wanted: ctx.wanted, target },
        &mut entry.rng,
    );
    // Early fire (aim awareness): one roll per awareness window for a ship that is on the aim line.
    let mut aim_window = 0;
    if p.aim_fire_probability > 0.0 {
        aim_window = world_aim_window;
        if step.state == State::Aim && entry.fire_rolled != aim_window {
            entry.fire_rolled = aim_window;
            let dist = (target.0 - ctx.ship.0).hypot(target.1 - ctx.ship.1);
            if tm::early_fire_in_range(dist, entry.st.reaches().0, p) && entry.rng.unit() < p.aim_fire_probability {
                entry.fire_until = now + f64::from(p.aim_fire_s);
            }
        }
        let want = now < entry.fire_until;
        if want != entry.fire_set {
            // SAFETY: page-checked read, then a one-byte write into the live creature (the same byte the player's mouse sets).
            unsafe {
                if rd::<u8>(ctx.creature + OFF_FIRE0).is_some() {
                    std::ptr::write_unaligned((ctx.creature + OFF_FIRE0) as *mut u8, u8::from(want));
                }
            }
            entry.fire_set = want;
            if want {
                ANY_FIRE.store(true, Ordering::Relaxed);
            }
        }
    }
    let _ = aim_window;
    let factor = match step.state {
        State::Dive => f32::from_bits(DIVE_BOOST.load(Ordering::Relaxed)),
        State::Aim => f32::from_bits(AIM_BOOST.load(Ordering::Relaxed)),
        _ => 1.0,
    };
    if factor > 1.0 || entry.boosting {
        entry.boosting = factor > 1.0;
        note_boost(ctx.creature, now, factor);
    }
    (step.waypoint.filter(|(x, y)| x.is_finite() && y.is_finite()), step.state)
}
