//! Aim-dodge and dive-avoidance game-memory glue around the pure `tactics_math`. Runs for every AI
//! ship on every AI tick, so the disabled path is one atomic load, and everything read from the game is
//! page-checked (`crate::rd`). Own gun reach for early fire is refreshed once a second.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::tactics_config::TacticsConfig;
use crate::tactics_math::{self as tm, Bounds, Rng, ShipState, State, StepInput, TacticsParams};
use crate::{plausible, rd, Ctx};

const OFF_AI_FLAG: usize = 0xbd4; // u8: 0 = flown by a player
const OFF_MODE: usize = 0xb48; // i32: 1 airborne combat, 3 landed, 0 takeoff (telemetry column `mode`; NOT +0xd70, that is the role)
/// Player fire buttons, sampled read-only for hostile aim awareness.
const OFF_FIRE0: usize = 0xbbc;
const OFF_NODE_ID: usize = 0x18;
const OFF_ACTIVE: usize = 0x48; // i32
const OFF_STATE: usize = 0xbdc; // i32 driver state, 6 / 7 = dead or gone
const OFF_CODE: usize = 0x28;
const OFF_REACH: usize = 0x1838; // f32 gun reach, +0x183c second fire group (read as a pair)
const OFF_AI_AIM: usize = 0xbf8; // [f32; 2] unit aim vector from AiWeaponController
const OFF_GUNS0_BEGIN: usize = 0x13a8;
const OFF_GUNS0_END: usize = 0x13b0;
const OFF_GUNS1_BEGIN: usize = 0x13c0;
const OFF_GUNS1_END: usize = 0x13c8;
const GUN_ENTRY_STRIDE: usize = 0x18;
const GUN_ENTRY_PTR: usize = 0x8;
const GUN_LOADED: usize = 0x860;
const GUN_RATE_TIMER: usize = 0x86c;
const GUN_BLOCK_TIMER: usize = 0x870;
const GUN_LATCH: usize = 0x874;
const MAX_GUNS_PER_GROUP: usize = 32;
/// Global creature list (Steam 1.163), stride 0x18, creature pointer at +8.
const CREATURES_BEGIN: usize = 0x1_43fa_a5d8;
const CREATURES_END: usize = 0x1_43fa_a5e0;
const CREATURE_STRIDE: usize = 0x18;
const CREATURE_PTR_OFF: usize = 0x8;
const MAX_LIVE_SHIPS: usize = 128;
const SHIP_SCAN_INTERVAL_S: f64 = 0.05;
/// Hold-over after a sampled player fire-button state (not confirmed shots).
const PLAYER_FIRE_HOLD_S: f64 = 0.6;
/// Arena extents the stock AI's map-edge push and the deploy code use: x max, y max, x min, y min (f32 each).
const BOUNDS_ADDRESS: usize = 0x1_403c_2bc0;

const REFRESH_S: f64 = 1.0;
const MAX_ENTRIES: usize = 256;
const MAX_GATE_LOGS: u32 = 24;

static ENABLED: AtomicBool = AtomicBool::new(false);
static PARAMS: OnceLock<TacticsParams> = OnceLock::new();
static GATE_LOGS: AtomicU32 = AtomicU32::new(0);
/// `dive_boost` as f32 bits (1.0 = none).
static DIVE_BOOST: AtomicU32 = AtomicU32::new(0x3f80_0000);
static AIM_BOOST: AtomicU32 = AtomicU32::new(0x3f80_0000);
/// A running dodge boost: the factor, and the world direction to dash in (`None` = strength only, e.g. a
/// flare-curtain retreat). Written by `decide`, read by the engine-command hook.
#[derive(Clone, Copy)]
struct Boost {
    creature: usize,
    /// The decision tick that wrote this entry (a later stage may re-point it in the same tick).
    noted_at: f64,
    until: f64,
    factor: f32,
    escape: Option<(f32, f32)>,
}
static BOOSTED: Mutex<Vec<Boost>> = Mutex::new(Vec::new());
/// The player's aim: two adjacent f32, a world-aligned direction (y not flipped), magnitude up to 100 (measured: points
/// at the target within ~3 deg while firing, see hf_notes findings/ai-telemetry.md).
const AIM_ADDRESS: usize = 0x1_403c_0dc8;
/// Aim vectors shorter than this are not trusted (no aim).
const MIN_AIM_LEN: f32 = 5.0;
/// How long one `dive` decision keeps the boost on (the engine commands go out right after the steering in the same tick).
const BOOST_HOLD_S: f64 = 0.3;

/// Boost factor and dash direction the engine-command hook applies to `creature` at `now` (`1.0` / `None` = none).
pub fn dodge_boost_for(creature: usize, now: f64) -> (f32, Option<(f32, f32)>) {
    let Ok(list) = BOOSTED.try_lock() else { return (1.0, None) };
    list.iter()
        .filter(|entry| entry.creature == creature && now <= entry.until)
        .fold((1.0, None), |best, entry| {
            if entry.factor > best.0 { (entry.factor, entry.escape) } else { best }
        })
}

/// Whether the engine hook must be installed for the dodge boost.
pub fn wants_engine_boost() -> bool {
    ENABLED.load(Ordering::Relaxed) && f32::from_bits(DIVE_BOOST.load(Ordering::Relaxed)).max(f32::from_bits(AIM_BOOST.load(Ordering::Relaxed))) > 1.0
}

/// Keeps a waypoint inside the arena (50 u margin at the sides and the ceiling) using the bounds the tactics stage
/// read; unchanged when they are not known (tactics off or no battle yet).
pub fn clamp_to_arena(waypoint: (f32, f32), ship: (f32, f32)) -> (f32, f32) {
    let Ok(guard) = WORLD.try_lock() else { return waypoint };
    let Some(bounds) = guard.as_ref().and_then(|w| w.bounds) else { return waypoint };
    tm::clamp_waypoint(waypoint, ship, &bounds)
}

/// Drops the chase boost of `creature` (the panic stage owns the engines from now on).
fn clear_boost(creature: usize) {
    let Ok(mut list) = BOOSTED.try_lock() else { return };
    list.retain(|entry| entry.creature != creature);
}

fn note_boost(creature: usize, now: f64, factor: f32, escape: Option<(f32, f32)>) {
    let Ok(mut list) = BOOSTED.try_lock() else { return };
    list.retain(|entry| entry.creature != creature && entry.until >= now - 5.0 && entry.until <= now + 5.0);
    if factor > 1.0 {
        list.push(Boost { creature, noted_at: now, until: now + BOOST_HOLD_S, factor, escape });
    }
}

/// Re-points a running boost at the waypoint the steering hook actually stores. The panic stage runs after
/// `decide` and can override the tactic's waypoint (curtain retreat, missile evasion), so the engine hook must
/// dash along the final choice, not the tactic's.
pub fn set_escape(creature: usize, now: f64, escape: Option<(f32, f32)>) {
    let Ok(mut list) = BOOSTED.try_lock() else { return };
    if let Some(entry) = list.iter_mut().rev().find(|entry| entry.creature == creature && entry.noted_at == now) {
        entry.escape = escape;
    }
}

struct Entry {
    node_id: u32,
    st: ShipState,
    rng: Rng,
    next_refresh: f64,
    boosting: bool,
    /// Early-fire range permission; never owns or writes a native fire byte.
    fire_rolled: u32,
    fire_until: f64,
    fire_target: Option<(usize, u32)>,
}


#[derive(Clone, Copy)]
struct LiveShip {
    creature: usize,
    side: f32,
    pos: (f32, f32),
    aim_dir: Option<(f32, f32)>,
    shooting: bool,
}

#[derive(Default)]
struct World {
    last_now: f64,
    last_prune: f64,
    ships: HashMap<usize, Entry>,
    bounds: Option<Bounds>,
    next_bounds_read: f64,
    bounds_logged: bool,
    live_ships: Vec<LiveShip>,
    next_ship_scan: f64,
    player_last_fire: f64,
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
            "hf_ai: tactics ON (allies {}; dive {} chance {} cone {} deg range {} dwell {} s boost {})",
            params.allies,
            params.dive_enabled,
            params.dive_chance,
            params.dive_cone_deg,
            params.dive_range,
            params.dive_dwell_s,
            params.dive_boost
        );
        if params.aim_enabled {
            log::info!(
                "hf_ai: aim awareness ON (chance {}, window {} s every {}..{} s, tolerance {} u, range {} u, weight {}, boost {})",
                params.aim_chance,
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
                    "hf_ai: early-fire range permission ON (chance {} per window, {} s, within {} x group-0 reach; native firing checks retained)",
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

unsafe fn gun_group_shooting(begin_off: usize, end_off: usize, creature: usize) -> bool {
    let (Some(begin), Some(end)) = (rd::<usize>(creature + begin_off), rd::<usize>(creature + end_off)) else {
        return false;
    };
    if begin == 0 || end <= begin {
        return false;
    }
    for entry in (begin..end).step_by(GUN_ENTRY_STRIDE).take(MAX_GUNS_PER_GROUP) {
        let Some(gun) = rd::<usize>(entry + GUN_ENTRY_PTR).filter(|&g| plausible(g)) else {
            continue;
        };
        if rd::<f32>(gun + GUN_RATE_TIMER).is_some_and(|rate| rate.is_finite() && rate > 0.0) {
            return true;
        }
        let loaded = rd::<f32>(gun + GUN_LOADED).unwrap_or(0.0);
        let block = rd::<f32>(gun + GUN_BLOCK_TIMER).unwrap_or(1.0);
        let latch = rd::<[u8; 2]>(gun + GUN_LATCH).unwrap_or([0, 0]);
        if loaded.is_finite() && loaded >= 1.0 && block.is_finite() && block <= 0.0 && (latch[0] != 0 || latch[1] != 0) {
            return true;
        }
    }
    false
}

unsafe fn ai_ship_shooting(creature: usize) -> bool {
    gun_group_shooting(OFF_GUNS0_BEGIN, OFF_GUNS0_END, creature)
        || gun_group_shooting(OFF_GUNS1_BEGIN, OFF_GUNS1_END, creature)
}

unsafe fn refresh_live_ships(world: &mut World, now: f64) {
    if now < world.next_ship_scan {
        return;
    }
    world.next_ship_scan = now + SHIP_SCAN_INTERVAL_S;
    world.live_ships.clear();
    let (Some(begin), Some(end)) = (rd::<usize>(CREATURES_BEGIN), rd::<usize>(CREATURES_END)) else {
        return;
    };
    if begin == 0 || end <= begin {
        return;
    }
    for slot in (begin..end).step_by(CREATURE_STRIDE).take(MAX_LIVE_SHIPS) {
        let Some(creature) = rd::<usize>(slot + CREATURE_PTR_OFF).filter(|&c| live_ship(c)) else {
            continue;
        };
        let Some(side) = rd::<f32>(creature + crate::OFF_SIDE).filter(|s| s.is_finite() && *s != 0.0) else {
            continue;
        };
        let (Some(x), Some(y)) = (rd::<f32>(creature + crate::OFF_POS_X), rd::<f32>(creature + crate::OFF_POS_Y)) else {
            continue;
        };
        if !(x.is_finite() && y.is_finite()) {
            continue;
        }
        let Some(ai_flag) = rd::<u8>(creature + OFF_AI_FLAG) else {
            continue;
        };
        let (aim_dir, shooting) = if ai_flag == 0 {
            let firing_now = rd::<[u8; 2]>(creature + OFF_FIRE0).is_some_and(|f| f[0] != 0 || f[1] != 0);
            if firing_now {
                world.player_last_fire = now;
            }
            let dir = rd::<[f32; 2]>(AIM_ADDRESS)
                .filter(|[ax, ay]| ax.is_finite() && ay.is_finite() && ax.hypot(*ay) >= MIN_AIM_LEN)
                .and_then(|[ax, ay]| tm::normalized(ax, ay));
            (dir, firing_now || (world.player_last_fire > 0.0 && now - world.player_last_fire <= PLAYER_FIRE_HOLD_S))
        } else {
            let dir = rd::<[f32; 2]>(creature + OFF_AI_AIM).and_then(|[ax, ay]| tm::normalized(ax, ay));
            (dir, ai_ship_shooting(creature))
        };
        world.live_ships.push(LiveShip {
            creature,
            side,
            pos: (x, y),
            aim_dir,
            shooting,
        });
    }
}

/// Picks the closest hostile shooter whose aim ray crosses `ship` within `p.aim_range` and `p.aim_tolerance`.
fn pick_hostile_aim(live_ships: &[LiveShip], creature: usize, side: f32, ship: (f32, f32), p: &TacticsParams) -> Option<tm::AimInput> {
    live_ships
        .iter()
        .filter(|s| s.creature != creature && s.side.signum() != side.signum() && (!p.aim_require_fire || s.shooting))
        .filter_map(|s| {
            let dir = s.aim_dir?;
            let cand = tm::AimInput { player: s.pos, dir };
            tm::aim_ray_hits(&cand, ship, p).then(|| ((s.pos.0 - ship.0).hypot(s.pos.1 - ship.1), cand))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, cand)| cand)
}

/// Called only at the native group-0 range gate, before its alignment/FF checks.
/// Stale identity, control handoffs, missing targets and stale steering fail closed.
pub fn early_fire_allowed(creature: usize, distance: f32) -> bool {
    let Some(p) = PARAMS.get().filter(|p| p.enabled && p.aim_fire_probability > 0.0) else {
        return false;
    };
    // SAFETY: borrowed scalar reads only, all page-checked; no game memory is mutated.
    let (node_id, now, target, reach) = unsafe {
        if !live_ship(creature) {
            return false;
        }
        if rd::<u8>(creature + OFF_AI_FLAG).unwrap_or(0) == 0 || rd::<i32>(creature + OFF_MODE) != Some(1) {
            return false;
        }
        if !crate::maneuvers_allowed(creature) {
            return false;
        }
        let Some(side) = rd::<f32>(creature + crate::OFF_SIDE).filter(|s| s.is_finite() && *s != 0.0) else {
            return false;
        };
        if side > 0.0 && !p.allies {
            return false;
        }
        let Some(node_id) = rd::<u32>(creature + OFF_NODE_ID) else { return false };
        let Some(now) = rd::<f64>(crate::CLOCK).filter(|t| t.is_finite()) else { return false };
        let Some((target, _)) = rd::<usize>(creature + crate::OFF_TARGET).and_then(|node| crate::target_info(node)) else {
            return false;
        };
        if target == creature || !live_ship(target) {
            return false;
        }
        let Some(target_id) = rd::<u32>(target + OFF_NODE_ID) else { return false };
        (node_id, now, (target, target_id), rd::<f32>(creature + OFF_REACH))
    };
    let Ok(guard) = WORLD.try_lock() else { return false };
    let Some(entry) = guard.as_ref().and_then(|w| w.ships.get(&creature)) else { return false };
    entry.node_id == node_id
        && entry.fire_target == Some(target)
        && now >= entry.st.last_seen
        && now - entry.st.last_seen <= BOOST_HOLD_S
        && now < entry.fire_until
        && tm::early_fire_in_range(distance, reach, p)
}

/// The tactics decision for one ship. Returns the waypoint (or `None` = leave the stock one) and the state for the log.
pub fn decide(ctx: &Ctx, panicking: impl FnOnce() -> bool) -> (Option<(f32, f32)>, State) {
    if !ENABLED.load(Ordering::Relaxed) {
        return (None, State::Stock);
    }
    if !ctx.now.is_finite() || !live_ship(ctx.creature) || !crate::maneuvers_allowed(ctx.creature) {
        return (None, State::Stock);
    }
    let Some(p) = PARAMS.get() else { return (None, State::Stock) };
    // Defensive aim awareness does not require the defender to own a target.
    if !ctx.side.is_finite() || ctx.side == 0.0 || (ctx.side > 0.0 && !p.allies) {
        return (None, State::Stock);
    }
    // SAFETY: page-checked reads.
    let (ai_flag, mode) = unsafe { (rd::<u8>(ctx.creature + OFF_AI_FLAG), rd::<i32>(ctx.creature + OFF_MODE)) };
    if ai_flag.unwrap_or(0) == 0 || mode != Some(1) {
        return (None, State::Stock);
    }
    if !(ctx.ship.0.is_finite() && ctx.ship.1.is_finite()) {
        return (None, State::Stock);
    }
    let target = ctx.target.zip(ctx.target_creature)
        .filter(|(pos, ship)| pos.0.is_finite() && pos.1.is_finite() && *ship != ctx.creature && live_ship(*ship));
    // SAFETY: page-checked scalar identity, matching plane_group's native node identity.
    let Some(node_id) = (unsafe { rd::<u32>(ctx.creature + OFF_NODE_ID) }) else { return (None, State::Stock) };
    // The panic query takes the ai_flares lock; it is called before ours and the two are never held together.
    let is_panicking = panicking();
    if is_panicking {
        clear_boost(ctx.creature);
        if let Ok(mut guard) = WORLD.try_lock() {
            if let Some(entry) = guard.as_mut().and_then(|w| w.ships.get_mut(&ctx.creature)) {
                entry.fire_until = 0.0;
                entry.boosting = false;
            }
        }
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
        crate::debug_state::clear();
        world.ships.clear();
        world.bounds = None;
        world.next_bounds_read = 0.0;
        world.bounds_logged = false;
        world.live_ships.clear();
        world.next_ship_scan = 0.0;
        world.player_last_fire = 0.0;
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

    if p.aim_enabled || p.aim_fire_probability > 0.0 {
        unsafe { refresh_live_ships(world, now) };
    }

    if world.ships.get(&ctx.creature).is_some_and(|entry| entry.node_id != node_id) {
        world.ships.remove(&ctx.creature);
    }
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
        let rng = Rng::new(seed);
        let st = ShipState::new(now);
        world.ships.insert(ctx.creature, Entry { node_id, st, rng, next_refresh: f64::NEG_INFINITY, boosting: false, fire_rolled: 0, fire_until: 0.0, fire_target: None });
    }
    let live_ships = &world.live_ships;
    let Some(entry) = world.ships.get_mut(&ctx.creature) else { return (None, State::Stock) };
    let window_open = (p.aim_enabled || p.aim_fire_probability > 0.0)
        && entry.st.aim_clock.update(now, p, &mut entry.rng);
    let aim = if p.aim_enabled && window_open {
        pick_hostile_aim(live_ships, ctx.creature, ctx.side, ctx.ship, p)
    } else {
        None
    };
    let ship_aim_window = entry.st.aim_clock.window;
    if now >= entry.next_refresh {
        let first = entry.next_refresh == f64::NEG_INFINITY;
        entry.next_refresh = now + REFRESH_S;
        let own = read_reach(ctx.creature);
        entry.st.latch_reach(own);
        if first && GATE_LOGS.fetch_add(1, Ordering::Relaxed) < MAX_GATE_LOGS {
            log::info!(
                "hf_ai: tactics ship {:#x}: gun reach {:?}",
                ctx.creature,
                own
            );
        }
    }
    let step = tm::step(
        p,
        &mut entry.st,
        &bounds,
        &StepInput { now, aim, ship: ctx.ship, wanted: ctx.wanted, target: target.map(|(pos, _)| pos) },
        &mut entry.rng,
    );
    // Grant range permission only. The native weapon controller owns firing and burst latches.
    let fire_target = target.and_then(|(_, ship)| {
        // SAFETY: page-checked scalar read from the validated target.
        unsafe { rd::<u32>(ship + OFF_NODE_ID) }.map(|id| (ship, id))
    });
    if entry.fire_target != fire_target {
        entry.fire_until = 0.0;
        entry.fire_target = fire_target;
    }
    if let Some((target_pos, _)) = target.filter(|_| fire_target.is_some() && p.aim_fire_probability > 0.0) {
        if window_open && entry.fire_rolled != ship_aim_window {
            entry.fire_rolled = ship_aim_window;
            let dist = (target_pos.0 - ctx.ship.0).hypot(target_pos.1 - ctx.ship.1);
            if tm::early_fire_in_range(dist, entry.st.reach(), p) && entry.rng.unit() < p.aim_fire_probability {
                entry.fire_until = now + f64::from(p.aim_fire_s);
            }
        }
    }
    let waypoint = step.waypoint.filter(|(x, y)| x.is_finite() && y.is_finite());
    let factor = match step.state {
        State::DiveDodge => f32::from_bits(DIVE_BOOST.load(Ordering::Relaxed)),
        State::Aim => f32::from_bits(AIM_BOOST.load(Ordering::Relaxed)),
        _ => 1.0,
    };
    if factor > 1.0 || entry.boosting {
        entry.boosting = factor > 1.0;
        // The engine hook dashes along the waypoint the tick is about to store, not the stock direction.
        let escape = waypoint.and_then(|(x, y)| tm::normalized(x - ctx.ship.0, y - ctx.ship.1));
        note_boost(ctx.creature, now, factor, escape);
    }
    (waypoint, step.state)
}
