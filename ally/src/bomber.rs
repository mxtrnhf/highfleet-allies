//! Ally-only physical ship bombs, not AircraftLogic loadouts. RE evidence and hook contracts:
//! docs/ship-bombers.md. All callbacks execute synchronously on the game's AI thread.
//! Unsafe helpers borrow live native nodes only for this callback: scalar reads are page-checked,
//! parent/vector walks are bounded, and no references or owning native values escape the call.

use std::mem::offset_of;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

use crate::battle;
use crate::bomber_math::{Flight, Phase, Run, PULL_OUT_S, RELEASE_HEIGHT};
use crate::diag::rd;
use crate::native::plausible;
use crate::native_layout::{Body, Creature, Link, Node};
use patchy::{PatchSession, ReturnType, Trampoline};

// After hf_ai's waypoint hook: an armed bomber keeps its course instead of evading missiles.
const WAYPOINT_SITE: usize = 0x1_4006_4a96;
const WAYPOINT_BYTES: [u8; 5] = [0xf3, 0x45, 0x0f, 0x5c, 0xcc]; // subss xmm9,xmm12
const STORE_WAYPOINT_X: [u8; 9] = [0xf3, 0x45, 0x0f, 0x11, 0x8e, 0xf0, 0x0b, 0, 0];
// Native desired speed in XMM12, after waypoint normalization and obstacle scanning.
const SPEED_SITE: usize = 0x1_4006_4f58;
const SPEED_BYTES: [u8; 5] = [0xf3, 0x0f, 0x10, 0x55, 0xa0]; // movss xmm2,[rbp-0x60]
                                                             // All four stack engine commands are initialized; hf_ai's boost hook is later at 0x140065744.
const ENGINE_SITE: usize = 0x1_4006_5734;
const ENGINE_BYTES: [u8; 7] = [0xc7, 0x45, 0x20, 0, 0, 0, 0]; // mov dword [rbp+0x20],0
                                                              // After hf_ai's engine hook, restore the native course before bomber thrust is applied.
const FINAL_ENGINE_SITE: usize = 0x1_4006_5750;
const FINAL_ENGINE_BYTES: [u8; 5] = [0x49, 0x8b, 0x4c, 0x38, 0x08];
const ENGINE_COMMANDS_SIZE: usize = 0x80;
const ENGINE_COMMANDS_PREFIX: usize = 0x40;
const BOMBER_BOOST: f32 = 2.0;
const BOOST_ON_S: f64 = 0.5;
const BOOST_CYCLE_S: f64 = 1.5;
const BOOST_TEMPERATURE_LIMIT: f32 = 350.0; // native engine ignition is 400, force lockout 500
                                            // CalculateStats 0x14008075d..0x140080785: max engine +0x2ec temperature / any +0x810 lock.
const OFF_MAX_ENGINE_TEMPERATURE: usize = 0x181c;
const OFF_ENGINE_LOCKED: usize = 0x1828;
/// A predicted impact inside the target's own bounding radius is a hit.
const RELEASE_TOLERANCE_RADII: f32 = 1.0;
/// Preferred minimum downward speed at release; scaled down when the ship cannot achieve it.
const MIN_RELEASE_DESCENT_SPEED: f32 = 80.0;
/// The steering hook applies the pull-out guard before the weapon hook runs, so a shot that
/// becomes valid on the same tick would be stolen. Accept release for this long after the
/// dive turns into Recover while the ship is still moving down.
const RECOVERY_RELEASE_GRACE_S: f64 = 0.5;
const AIM_MISS_GAIN: f32 = 0.5;
const AIM_MISS_GAIN_FAR: f32 = 1.2;
const AIM_MISS_DAMPING_S: f32 = 0.75;
const MAX_LATERAL_WAYPOINT: f32 = 160.0;
const RELEASE_SITE: usize = 0x1_4007_bf15;
const RELEASE_BYTES: [u8; 5] = [0xe8, 0x66, 0x05, 0xff, 0xff];
const STOCK_RELEASE: usize = 0x1_4006_c480;
const DETACH_BOMB: usize = 0x1_4002_e7d0;
const FIRE_FLARE: usize = 0x1_4002_eed0;
const WORLD_POSITION: usize = 0x1_4004_8b60;
const PREDICT_HIT: usize = 0x1_401b_5420;
const FRAME_RATE: usize = 0x1_403c_0de4;
const OFF_BOMB_RADIUS: usize = 0x20c;
const OFF_MASTER: usize = offset_of!(Node, master) + offset_of!(Link, target);
const OFF_OWNER: usize = offset_of!(Node, owner) + offset_of!(Link, target);
const OFF_BOMBS: usize = offset_of!(Creature, bombs);
const OFF_FLARES: usize = offset_of!(Creature, flare_launchers);
const FLARE_ENTRY_SIZE: usize = std::mem::size_of::<Link>();
const FLARE_ENTRY_PART: usize = offset_of!(Link, target);
/// Ceiling on the flare launchers walked in one pulse; a longer vector means the header is corrupt.
const FLARE_LIMIT: usize = 256;
const OFF_RADIUS: usize = offset_of!(Creature, bounding_radius);
const OFF_MASS: usize = offset_of!(Creature, mass);
const OFF_THRUST: usize = offset_of!(Creature, thrust_group_0);
const OFF_WAYPOINT: usize = offset_of!(Creature, waypoint_x);
const DROP_INTERVAL_S: f64 = 0.05;
const FLARE_INTERVAL_S: f64 = 0.5;
const MAX_RUNS: usize = 32;

const _: () = {
    assert!(offset_of!(Body, velocity_x) == 0x130);
    assert!(offset_of!(Body, velocity_y) == 0x134);
    assert!(offset_of!(Body, angle) == 0x138);
    assert!(OFF_MASTER == 0x88);
    assert!(OFF_OWNER == 0xb8);
};

static WAYPOINT: [AtomicU32; 2] = [AtomicU32::new(0), AtomicU32::new(0)];
static WANTED_SPEED: AtomicU32 = AtomicU32::new(0);
static RUNS: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
static FAILED: AtomicBool = AtomicBool::new(false);
static NEXT_GATE_LOG: AtomicU64 = AtomicU64::new(0);

/// Stack-only native output, NOT a game-owned object. Verified 0x1401b5420 writes +0/+4.
#[repr(C)]
struct Prediction {
    crosses_target: u8,
    padding: [u8; 3],
    horizontal_miss: f32,
}

const _: () = {
    assert!(std::mem::size_of::<Prediction>() == 8);
    assert!(offset_of!(Prediction, horizontal_miss) == 4);
};

pub fn reset() {
    NEXT_GATE_LOG.store(0, Ordering::Relaxed);
    if let Ok(mut runs) = RUNS.try_lock() {
        runs.clear();
    }
}

struct Entry {
    ship: usize,
    target: usize,
    run: Run,
    now: f64,
    waypoint: (f32, f32),
    last_drop: f64,
    last_flare: f64,
    last_prediction_log: f64,
    last_boost_log: f64,
    last_boost_phase: Option<Phase>,
    wanted_speed: f32,
    native_commands: [u8; ENGINE_COMMANDS_SIZE],
    commands_valid: bool,
    next_aim: f64,
    aim_miss: f32,
    last_aim_time: f64,
    aim_horizontal: f32,
    missile_block_logged: bool,
    boost_started: f64,
    course_active: bool,
    recovery_target: ((f32, f32), f32),
    /// A salvo started during the post-pull-out grace; release the rest without re-diving.
    releasing: bool,
}

unsafe fn boost_available(ship: usize) -> bool {
    rd::<f32>(ship + OFF_MAX_ENGINE_TEMPERATURE)
        .is_some_and(|temperature| temperature.is_finite() && temperature < BOOST_TEMPERATURE_LIMIT)
        && rd::<u8>(ship + OFF_ENGINE_LOCKED) == Some(0)
}

unsafe fn live(ship: usize) -> bool {
    plausible(ship)
        && rd::<u8>(ship + crate::native::OFF_CODE).is_some_and(|code| code & 0x2f == 0x2f)
        && rd::<i32>(ship + offset_of!(Node, state)).is_some_and(|active| active != 0)
        && rd::<i32>(ship + crate::native::OFF_STATE).is_some_and(|state| !matches!(state, 6 | 7))
}

unsafe fn landed_enemy(target: usize) -> bool {
    live(target)
        && rd::<f32>(target + crate::native::OFF_SIDE)
            .is_some_and(|side| side.is_finite() && side < 0.0)
        && rd::<u8>(target + crate::native::OFF_PRIZE) == Some(0)
        && rd::<i32>(target + crate::native::OFF_MODE) == Some(3)
}

/// Preserve native target-link registration; no raw owning Link writes.
unsafe fn set_target(ship: usize, target: usize) {
    let set_link: crate::native::SetLinkFn = std::mem::transmute(crate::native::SET_LINK);
    // SAFETY: synchronous AI callback, validated live ship/target, native borrowed Link;
    // native SetLink updates registration and retains all allocation/ownership handling.
    set_link(ship + crate::native::OFF_TARGET_LINK, target);
}

/// Bounded equivalent of Node_GetRootPart: only follow Body masters, not owner links.
pub(crate) unsafe fn root_part(mut node: usize) -> Option<usize> {
    for _ in 0..64 {
        if !plausible(node) {
            return None;
        }
        let parent: usize = rd(node + OFF_MASTER)?;
        if parent == 0 {
            return Some(node);
        }
        if !plausible(parent) {
            return None;
        }
        if rd::<u8>(parent + crate::native::OFF_CODE)? & 0xf != 0xf {
            return Some(node);
        }
        node = parent;
    }
    None
}

/// Borrow the first still-attached bomb. Never copy or mutate the game's owning vector.
unsafe fn first_bomb(ship: usize) -> Option<usize> {
    let begin: usize = rd(ship + OFF_BOMBS)?;
    let end: usize = rd(ship + OFF_BOMBS + 8)?;
    let length = end.checked_sub(begin)?;
    if !plausible(begin)
        || length % std::mem::size_of::<Link>() != 0
        || length / std::mem::size_of::<Link>() > 256
    {
        return None;
    }
    let root = root_part(ship)?;
    for address in (begin..end).step_by(std::mem::size_of::<Link>()) {
        let bomb: usize = rd(address + offset_of!(Link, target))?;
        if plausible(bomb)
            && rd::<usize>(bomb + OFF_OWNER) == Some(ship)
            && rd::<i32>(bomb + offset_of!(Node, state)).is_some_and(|active| active != 0)
            && root_part(bomb) == Some(root)
        {
            return Some(bomb);
        }
    }
    None
}

unsafe fn flight(ship: usize) -> Option<(usize, Flight)> {
    if !live(ship)
        || rd::<u8>(ship + crate::native::OFF_AI_FLAG)? == 0
        || rd::<i32>(ship + crate::native::OFF_MODE)? != 1
        || rd::<i32>(ship + crate::native::OFF_STATE)? != 1
        || !rd::<f32>(ship + crate::native::OFF_SIDE)
            .is_some_and(|side| side.is_finite() && side > 0.0)
    {
        return None;
    }
    // Post-salvo escape must not vanish when the bomb destroys the target or native AI
    // retargets. Retain scalar geometry only; never dereference a cached dead target.
    let recovery = RUNS.try_lock().ok().and_then(|runs| {
        runs.iter()
            .find(|entry| {
                entry.ship == ship && matches!(entry.run.phase, Phase::Recover | Phase::Done)
            })
            .map(|entry| (entry.target, entry.recovery_target))
    });
    let (target, target_position, target_radius) =
        if let Some((target, (position, radius))) = recovery {
            (target, [position.0, position.1], radius)
        } else {
            let node: usize = rd(ship + crate::native::OFF_TARGET)?;
            if !plausible(node) {
                return None;
            }
            let target = if rd::<u8>(node + crate::native::OFF_CODE)? & 0x2f == 0x2f {
                node
            } else {
                rd(node + OFF_OWNER)?
            };
            if !landed_enemy(target) {
                return None;
            }
            (
                target,
                rd::<[f32; 2]>(target + crate::native::OFF_POS_X)?,
                rd::<f32>(target + OFF_RADIUS)?,
            )
        };
    let root = root_part(ship)?;
    let position: [f32; 2] = rd(ship + crate::native::OFF_POS_X)?;
    let velocity: [f32; 2] = rd(root + offset_of!(Body, velocity_x))?;
    let own_radius: f32 = rd(ship + OFF_RADIUS)?;
    let mass: f32 = rd(ship + OFF_MASS)?;
    let thrust: [f32; 2] = rd(ship + OFF_THRUST)?;
    let now: f64 = rd(crate::native::CLOCK)?;
    if !now.is_finite()
        || !position
            .into_iter()
            .chain(target_position)
            .chain(velocity)
            .chain([own_radius, target_radius, mass, thrust[0], thrust[1]])
            .all(f32::is_finite)
        || mass <= 0.0
        || own_radius <= 0.0
        || target_radius <= 0.0
    {
        return None;
    }
    let available_strength = if boost_available(ship) {
        BOMBER_BOOST
    } else {
        1.0
    };
    // Available lift with a safety factor, not half of it: the earlier 0.5 factor made the
    // permitted descent so slow that release windows were impractical.
    let recovery_acceleration = ((thrust[0] + thrust[1]) * available_strength / mass - 9.82) * 0.8;
    if recovery_acceleration <= 1.0 || !recovery_acceleration.is_finite() {
        return None; // insufficient lift: never initiate a dive we cannot pull out of
    }
    Some((
        target,
        Flight {
            now,
            position: (position[0], position[1]),
            target: (target_position[0], target_position[1]),
            velocity: (velocity[0], velocity[1]),
            own_radius,
            target_radius,
            recovery_acceleration,
        },
    ))
}

/// Debug-only eligibility snapshot: a disabled gate must not look like a broken hook.
unsafe fn log_gate(ship: usize) {
    if !crate::diag::debug() {
        return;
    }
    let Some(now) = rd::<f64>(crate::native::CLOCK).filter(|now| now.is_finite()) else {
        return;
    };
    let next = f64::from_bits(NEXT_GATE_LOG.load(Ordering::Relaxed));
    if now < next && next - now < 10.0 {
        return;
    }
    NEXT_GATE_LOG.store((now + 3.0).to_bits(), Ordering::Relaxed);
    let node = rd::<usize>(ship + crate::native::OFF_TARGET).filter(|node| plausible(*node));
    let target = node.and_then(|node| {
        if rd::<u8>(node + crate::native::OFF_CODE)? & 0x2f == 0x2f {
            Some(node)
        } else {
            rd::<usize>(node + OFF_OWNER).filter(|target| plausible(*target))
        }
    });
    log::info!(
        "ALLY bomber gate: c={ship:#x} flight={:?} state={:?} ai={:?} bomb={:?} target={target:?} target_flight={:?} prize={:?} mass={:?} thrust={:?}",
        rd::<i32>(ship + crate::native::OFF_MODE), rd::<i32>(ship + crate::native::OFF_STATE),
        rd::<u8>(ship + crate::native::OFF_AI_FLAG), first_bomb(ship),
        target.and_then(|target| rd::<i32>(target + crate::native::OFF_MODE)),
        target.and_then(|target| rd::<u8>(target + crate::native::OFF_PRIZE)),
        rd::<f32>(ship + OFF_MASS), rd::<[f32; 2]>(ship + OFF_THRUST),
    );
}

/// Pulse every attached launcher through the native release path; ammo and per-launcher
/// cooldown gates remain native. Called only for an active run that still carries bombs.
unsafe fn drop_flares(ship: usize, entry: &mut Entry, now: f64) {
    if now - entry.last_flare < FLARE_INTERVAL_S {
        return;
    }
    entry.last_flare = now;
    let (Some(begin), Some(end)) = (
        rd::<usize>(ship + OFF_FLARES),
        rd::<usize>(ship + OFF_FLARES + 8),
    ) else {
        return;
    };
    if begin == end {
        return;
    }
    let Some(length) = end.checked_sub(begin) else {
        return;
    };
    if !plausible(begin) || !length.is_multiple_of(FLARE_ENTRY_SIZE) {
        return;
    }
    let flare_count = length / FLARE_ENTRY_SIZE;
    if flare_count > FLARE_LIMIT {
        return;
    }
    let Some(root) = root_part(ship) else {
        return;
    };
    let fire: unsafe extern "C" fn(usize) -> u8 = std::mem::transmute(FIRE_FLARE);
    let mut fired = 0;
    for address in (begin..end).step_by(FLARE_ENTRY_SIZE) {
        let Some(launcher) =
            rd::<usize>(address + FLARE_ENTRY_PART).filter(|part| plausible(*part))
        else {
            continue;
        };
        if rd::<usize>(launcher + OFF_OWNER) != Some(ship)
            || rd::<i32>(launcher + offset_of!(Node, state)).is_none_or(|active| active == 0)
            || root_part(launcher) != Some(root)
        {
            continue;
        }
        fired += usize::from(fire(launcher) != 0);
    }
    if fired != 0 {
        log::info!("ALLY bomber FLARES: c={ship:#x} fired {fired} launcher(s)");
    }
}

/// Continue the run's horizontal motion while pulling up; do not reverse toward/away from
/// another ship simply because the payload was released.
unsafe fn begin_escape(entry: &mut Entry, flight: &Flight) {
    entry.run.recover(flight.now);
    // A release callback can change phase after steering. Do not reuse the old downward
    // waypoint/engine snapshot as a recovery command before the next steering callback.
    entry.course_active = false;
    entry.commands_valid = false;
    entry.recovery_target = (flight.target, flight.target_radius);
    if flight.velocity.0.abs() >= 1.0 {
        entry.run.direction = flight.velocity.0.signum();
    } else {
        let course_x = entry.waypoint.0 - flight.position.0;
        if course_x != 0.0 {
            entry.run.direction = course_x.signum();
        }
    }
    log::info!(
        "ALLY bomber: c={:#x} pulling up/{} along run direction, minimum 1.5s until descent arrested",
        entry.ship,
        if entry.run.direction > 0.0 {
            "right"
        } else {
            "left"
        }
    );
}

unsafe fn steer(ship: usize) {
    if FAILED.load(Ordering::Relaxed) || !battle::tracked(ship) {
        return;
    }
    // Native target selection runs periodically. Keep a valid bombing target stable, especially
    // while recovering, rather than restarting a dive whenever that selector changes its mind.
    if first_bomb(ship).is_some()
        && live(ship)
        && rd::<u8>(ship + crate::native::OFF_AI_FLAG).is_some_and(|flag| flag != 0)
        && rd::<i32>(ship + crate::native::OFF_STATE) == Some(1)
        && rd::<i32>(ship + crate::native::OFF_MODE) == Some(1)
    {
        let desired = RUNS.try_lock().ok().and_then(|runs| {
            runs.iter()
                .find(|entry| {
                    entry.ship == ship && matches!(entry.run.phase, Phase::Dive | Phase::Salvo)
                })
                .map(|entry| entry.target)
        });
        if let Some(target) = desired.filter(|target| landed_enemy(*target)) {
            if rd::<usize>(ship + crate::native::OFF_TARGET) != Some(target) {
                set_target(ship, target);
            }
        }
    }
    let Some((target, flight)) = flight(ship) else {
        log_gate(ship);
        if let Ok(mut runs) = RUNS.try_lock() {
            runs.retain(|entry| entry.ship != ship);
        }
        return;
    };
    let has_bomb = first_bomb(ship).is_some();
    if !has_bomb {
        log_gate(ship);
    }
    let Ok(mut runs) = RUNS.try_lock() else {
        return;
    };
    runs.retain(|entry| flight.now >= entry.now && flight.now - entry.now < 5.0);
    let index = match runs.iter().position(|entry| entry.ship == ship) {
        Some(index) => index,
        None if runs.len() < MAX_RUNS && has_bomb => {
            let index = runs.len();
            runs.push(Entry {
                ship,
                target,
                run: Run::new(&flight),
                now: flight.now,
                waypoint: flight.position,
                last_drop: flight.now - DROP_INTERVAL_S,
                last_flare: flight.now - FLARE_INTERVAL_S,
                last_prediction_log: flight.now - 1.0,
                last_boost_log: flight.now - 1.0,
                last_boost_phase: None,
                wanted_speed: 0.0,
                native_commands: [0; ENGINE_COMMANDS_SIZE],
                commands_valid: false,
                next_aim: flight.now,
                aim_miss: 0.0,
                last_aim_time: flight.now,
                aim_horizontal: 0.0,
                missile_block_logged: false,
                boost_started: flight.now,
                course_active: false,
                recovery_target: (flight.target, flight.target_radius),
                releasing: false,
            });
            log::info!(
                "ALLY bomber: c={ship:#x} target={target:#x} starting speed-building approach; start_y={:.1}",
                flight.position.1,
            );
            index
        }
        _ => return,
    };
    let Some(entry) = runs.get_mut(index) else {
        return;
    };
    entry.course_active = false;
    entry.commands_valid = false;
    if entry.target != target {
        entry.target = target;
        if matches!(entry.run.phase, Phase::Dive | Phase::Salvo) {
            entry.run = Run::new(&flight);
        }
        entry.aim_miss = 0.0;
        entry.last_aim_time = flight.now;
        entry.aim_horizontal = 0.0;
        entry.next_aim = flight.now;
        entry.releasing = false;
    }
    if !has_bomb && entry.run.phase != Phase::Recover {
        return;
    }
    let previous = entry.run.phase;
    entry.now = flight.now;
    entry.waypoint = entry.run.waypoint(&flight);
    if entry.run.phase == Phase::Done {
        return;
    }
    if entry.run.phase == Phase::Recover && previous != Phase::Recover {
        log::info!(
            "ALLY bomber abort: c={ship:#x} {:?}, y={:.1}, elapsed={:.1}s, miss={:.1}; unsafe clearance, pulling up",
            previous, flight.position.1, flight.now - entry.run.started, entry.aim_miss,
        );
        begin_escape(entry, &flight);
        entry.waypoint = entry.run.waypoint(&flight);
    }
    if matches!(entry.run.phase, Phase::Dive | Phase::Salvo) && flight.now >= entry.next_aim {
        entry.next_aim = flight.now + 0.1;
        if let Some(prediction) =
            first_bomb(ship).and_then(|bomb| predict_bomb(ship, target, bomb, &flight))
        {
            let miss = prediction.horizontal_miss.clamp(-1000.0, 1000.0);
            let elapsed = (flight.now - entry.last_aim_time) as f32;
            let miss_rate = if elapsed > 0.0 {
                (miss - entry.aim_miss) / elapsed
            } else {
                0.0
            };
            // Full correction while the shot is still far off; only damp close to the target,
            // where the derivative term prevents overshoot. The old fixed gain was too weak to
            // converge before the descent reached the pull-out limit.
            let tolerance = flight.target_radius * RELEASE_TOLERANCE_RADII;
            let near = miss.abs() <= tolerance * 4.0;
            let gain = if near {
                AIM_MISS_GAIN
            } else {
                AIM_MISS_GAIN_FAR
            };
            let damping = if near { AIM_MISS_DAMPING_S } else { 0.0 };
            entry.aim_horizontal = -(gain * miss + damping * miss_rate)
                .clamp(-MAX_LATERAL_WAYPOINT, MAX_LATERAL_WAYPOINT);
            entry.aim_miss = miss;
            entry.last_aim_time = flight.now;
        }
    }
    if matches!(entry.run.phase, Phase::Dive | Phase::Salvo) {
        // Native signed miss includes launch velocity/impulse. Correct lateral drift during
        // descent rather than waiting indefinitely for an accurate release at a fixed height.
        entry.waypoint.0 = flight.position.0 + entry.aim_horizontal;
    }
    if entry.run.phase != Phase::Recover && !has_bomb {
        return;
    }
    if has_bomb {
        drop_flares(ship, entry, flight.now);
    }
    if previous != entry.run.phase {
        log::info!(
            "ALLY bomber: c={ship:#x} target={target:#x} {:?} -> {:?}, descent {:.1}",
            previous,
            entry.run.phase,
            flight.velocity.1
        );
    }
    WAYPOINT[0].store(entry.waypoint.0.to_bits(), Ordering::Relaxed);
    WAYPOINT[1].store(entry.waypoint.1.to_bits(), Ordering::Relaxed);
    entry.course_active = true;
}

unsafe extern "C" fn on_steer(ship: usize) {
    // SAFETY: synchronous native AI hook; only scalar borrowed reads, no game-owned Rust values.
    if catch_unwind(AssertUnwindSafe(|| steer(ship))).is_err() {
        FAILED.store(true, Ordering::Relaxed);
        log::error!("hf_ally: panic in bomber steering; ship bombers disabled");
    }
}

/// Only the same tick's unchanged bomber course owns speed/thrust. Our steering hook runs
/// after hf_ai while armed, so missile evasion cannot replace the waypoint or cancel release.
unsafe fn owns_course(entry: &Entry, target: usize, now: f64) -> bool {
    entry.course_active
        && entry.target == target
        && entry.now == now
        && rd::<[f32; 2]>(entry.ship + OFF_WAYPOINT) == Some([entry.waypoint.0, entry.waypoint.1])
}

unsafe fn raise_dive_speed(ship: usize) {
    if FAILED.load(Ordering::Relaxed) || !battle::tracked(ship) {
        return;
    }
    let Some((target, flight)) = flight(ship) else {
        return;
    };
    let Ok(mut runs) = RUNS.try_lock() else {
        return;
    };
    let Some(entry) = runs.iter_mut().find(|entry| entry.ship == ship) else {
        return;
    };
    if !owns_course(entry, target, flight.now) {
        return;
    }
    let speed = f32::from_bits(WANTED_SPEED.load(Ordering::Relaxed));
    if !speed.is_finite() || speed < 0.0 {
        return;
    }
    // Raising thrust alone still drives the stock ~80 u/s cruise governor. Raise its target
    // throughout the dive; recovery uses the native target. Native avoidance, feedback, gravity
    // compensation, attitude correction, bank limits and smoothing still run on the request.
    let wanted = if matches!(entry.run.phase, Phase::Dive | Phase::Salvo) {
        speed * BOMBER_BOOST
    } else {
        speed
    };
    if wanted.is_finite() {
        entry.wanted_speed = wanted;
        WANTED_SPEED.store(wanted.to_bits(), Ordering::Relaxed);
    }
}

unsafe extern "C" fn on_speed(ship: usize) {
    // SAFETY: this callback borrows scalar state during the synchronous AI tick. The scratch
    // value is XMM12 from this same tick; no pointer or native-owned object is retained.
    if catch_unwind(AssertUnwindSafe(|| raise_dive_speed(ship))).is_err() {
        FAILED.store(true, Ordering::Relaxed);
        log::error!("hf_ally: panic in bomber speed control; ship bombers disabled");
    }
}

unsafe fn boost_engines(ship: usize, command: usize) {
    if FAILED.load(Ordering::Relaxed) || !battle::tracked(ship) {
        return;
    }
    let Some((target, flight)) = flight(ship) else {
        return;
    };
    let Ok(mut runs) = RUNS.try_lock() else {
        return;
    };
    let Some(entry) = runs.iter_mut().find(|entry| entry.ship == ship) else {
        return;
    };
    if !owns_course(entry, target, flight.now)
        || rd::<i32>(command) != Some(0x66)
        || rd::<i32>(command + 0x20) != Some(0x67)
    {
        return;
    }
    // Plain stack command bytes, saved before our/hf_ai's writes for the later priority hook.
    let Some(command_start) = command.checked_sub(ENGINE_COMMANDS_PREFIX) else {
        return;
    };
    if rd::<i32>(command_start) != Some(0x64) || rd::<i32>(command_start + 0x20) != Some(0x65) {
        return;
    }
    let Some(native_commands) = rd::<[u8; ENGINE_COMMANDS_SIZE]>(command_start) else {
        return;
    };
    entry.native_commands = native_commands;
    entry.commands_valid = true;
    let (Some(first), Some(second)) = (rd::<f32>(command + 4), rd::<f32>(command + 0x24)) else {
        return;
    };
    if !first.is_finite() || !second.is_finite() || first < 0.0 || second < 0.0 {
        return;
    }
    // Continuous base thrust with timed boost pulses through the aiming approach/salvo.
    // Recovery commands full upward/away boost to brake descent; it is not cruise feedback.
    let diving = matches!(entry.run.phase, Phase::Dive | Phase::Salvo);
    let boost_ready = boost_available(ship);
    let dive_strength = if boost_ready
        && (flight.now - entry.boost_started).rem_euclid(BOOST_CYCLE_S) < BOOST_ON_S
    {
        BOMBER_BOOST
    } else {
        1.0
    };
    let recovery_strength = if boost_ready { BOMBER_BOOST } else { 1.0 };
    let boosted_first = if diving {
        dive_strength
    } else {
        recovery_strength
    };
    let boosted_second = if diving {
        dive_strength
    } else {
        recovery_strength
    };
    // SAFETY: the byte-validated 0x140065734 hook passes RBP+0x28: two initialized 0x20-byte
    // commands in this native invocation's writable stack frame. IDs were rechecked above.
    // These unaligned scalar writes end before native dispatch; no references/ownership escape.
    ((command + 4) as *mut f32).write_unaligned(boosted_first);
    ((command + 0x24) as *mut f32).write_unaligned(boosted_second);
    let mut commanded_angle = 0.0;
    if diving || entry.run.phase == Phase::Recover {
        // Native engine angle convention is atan2(world_x, -world_y), as used by
        // hf_ai::ai_flares::spin. Both banks share the approach or upward braking vector.
        let angle =
            (entry.waypoint.0 - flight.position.0).atan2(-(entry.waypoint.1 - flight.position.1));
        commanded_angle = angle;
        // SAFETY: same validated writable 0x80-byte native command block as above. Commands
        // 0x64/0x65 at command_start/+0x20 steer swivels with angle at +4. Force commands
        // 0x66/0x67 use +0x10. The player sends the SAME angle to both kinds every tick.
        // Changing only force angles leaves nozzles following stock AI, and cos(error)<=0
        // can cut thrust completely (Engine_ServoUpdate / Engine_ApplyThrustForce).
        ((command_start + 4) as *mut f32).write_unaligned(angle);
        ((command_start + 0x24) as *mut f32).write_unaligned(angle);
        ((command + 0x10) as *mut f32).write_unaligned(angle);
        ((command + 0x30) as *mut f32).write_unaligned(angle);
    }
    if entry.last_boost_phase != Some(entry.run.phase) || flight.now - entry.last_boost_log >= 1.0 {
        entry.last_boost_log = flight.now;
        entry.last_boost_phase = Some(entry.run.phase);
        log::info!(
            "ALLY bomber BOOST: c={ship:#x} {:?} target_speed={:.1} descent={:.1} lateral={:.1} servo_and_force_angle={commanded_angle:.3} boost_ready={boost_ready} temperature={:?} bank0={first:.2}->{boosted_first:.2} bank1={second:.2}->{boosted_second:.2}",
            entry.run.phase, entry.wanted_speed, flight.velocity.1, flight.velocity.0,
            rd::<f32>(ship + OFF_MAX_ENGINE_TEMPERATURE),
        );
    }
}

unsafe extern "C" fn on_engines(ship: usize, command: usize) {
    // SAFETY: command is borrowed from the active AI tick's stack; see boost_engines.
    if catch_unwind(AssertUnwindSafe(|| boost_engines(ship, command))).is_err() {
        FAILED.store(true, Ordering::Relaxed);
        log::error!("hf_ally: panic in bomber engine control; ship bombers disabled");
    }
}

/// hf_ai's engine hook has returned. Restore this tick's native stack commands before applying
/// bomber thrust, so panic boost/spin cannot take over an armed bombing or recovery course.
unsafe fn restore_engines(ship: usize, command: usize) {
    if FAILED.load(Ordering::Relaxed) || !battle::tracked(ship) {
        return;
    }
    let Some((target, flight)) = flight(ship) else {
        return;
    };
    let Ok(runs) = RUNS.try_lock() else {
        return;
    };
    let Some(entry) = runs.iter().find(|entry| entry.ship == ship) else {
        return;
    };
    if !owns_course(entry, target, flight.now) || !entry.commands_valid {
        return;
    }
    let Some(command_start) = command.checked_sub(ENGINE_COMMANDS_PREFIX) else {
        return;
    };
    if rd::<[u8; ENGINE_COMMANDS_SIZE]>(command_start).is_none() {
        return;
    }
    let native_commands = entry.native_commands;
    drop(runs);
    // SAFETY: the byte-validated 0x140065750 hook receives the same writable native stack
    // commands snapshot at 0x140065734. The copy is plain stack data, not a native owner.
    (command_start as *mut [u8; ENGINE_COMMANDS_SIZE]).write_unaligned(native_commands);
    boost_engines(ship, command);
}

unsafe extern "C" fn on_final_engines(ship: usize, command: usize) {
    // SAFETY: command is borrowed from the active AI tick's stack; see restore_engines.
    if catch_unwind(AssertUnwindSafe(|| restore_engines(ship, command))).is_err() {
        FAILED.store(true, Ordering::Relaxed);
        log::error!("hf_ally: panic in final bomber engine control; ship bombers disabled");
    }
}

/// Run the same predictor and launch-velocity calculation used by Creature_TryDropBomb.
/// Safety: ship/target/bomb are live, validated native nodes borrowed by this AI callback;
/// the bomb remains attached. Native code owns its temporary allocations and frees them.
unsafe fn predict_bomb(
    ship: usize,
    target: usize,
    bomb: usize,
    flight: &Flight,
) -> Option<Prediction> {
    let begin: usize = rd(crate::native::CREATURES_BEGIN)?;
    let end: usize = rd(crate::native::CREATURES_END)?;
    let length = end.checked_sub(begin)?;
    let frame_rate: f32 = rd(FRAME_RATE)?;
    if !plausible(begin)
        || length % std::mem::size_of::<Link>() != 0
        || length / std::mem::size_of::<Link>() > 4096
        || !frame_rate.is_finite()
        || !(1.0..=1000.0).contains(&frame_rate)
    {
        return None;
    }
    let get_position: unsafe extern "C" fn(usize, *mut [f32; 2]) -> usize =
        std::mem::transmute(WORLD_POSITION);
    let mut position = [0.0; 2];
    get_position(bomb, &mut position);
    let radius: f32 = rd(bomb + OFF_BOMB_RADIUS)?;
    if !radius.is_finite() || radius < 0.0 {
        return None;
    }
    // Native release starts below the bomb body, not at the carrier's centre of mass.
    position[1] += radius + 5.0;
    let profile: unsafe extern "C" fn() -> usize = std::mem::transmute(crate::native::GET_PROFILE);
    let ejection_speed: f32 = rd(profile() + 0xac)?;
    let root = root_part(ship)?;
    let angle: f32 = rd(root + offset_of!(Body, angle))?;
    let velocity = [
        flight.velocity.0 - angle.sin() * ejection_speed,
        flight.velocity.1 + angle.cos() * ejection_speed,
    ];
    if !angle.is_finite()
        || !ejection_speed.is_finite()
        || ejection_speed < 0.0
        || !position.into_iter().chain(velocity).all(f32::is_finite)
    {
        return None;
    }
    let mut prediction = Prediction {
        crosses_target: 0,
        padding: [0; 3],
        horizontal_miss: f32::MAX,
    };
    // RCX output, RDX target, R8 origin, R9 velocity, stack +0x20 faction, +0x28 ignored ship.
    // The native integrator includes gravity, drag, target motion and same-side obstruction.
    let predict: unsafe extern "C" fn(
        *mut Prediction,
        usize,
        *const [f32; 2],
        *const [f32; 2],
        f32,
        usize,
    ) -> *mut Prediction = std::mem::transmute(PREDICT_HIT);
    predict(
        &mut prediction,
        target,
        &position,
        &velocity,
        rd(ship + crate::native::OFF_SIDE)?,
        ship,
    );
    (prediction.horizontal_miss.is_finite() && prediction.horizontal_miss != f32::MAX)
        .then_some(prediction)
}

unsafe fn release(ship: usize) {
    let Some((target, flight)) = flight(ship) else {
        return;
    };
    let Ok(mut runs) = RUNS.try_lock() else {
        return;
    };
    let Some(entry) = runs
        .iter_mut()
        .find(|entry| entry.ship == ship && entry.target == target)
    else {
        return;
    };
    // A bypassed hook, new target or stale decision cancels release rather than dropping along
    // an old bombing course. While armed, our later steering hook deliberately overrides hf_ai.
    let recovery_grace = entry.run.phase == Phase::Recover
        && flight.velocity.1 > 0.0
        && entry.run.recover_until - flight.now > PULL_OUT_S - RECOVERY_RELEASE_GRACE_S;
    let accepting =
        matches!(entry.run.phase, Phase::Dive | Phase::Salvo) || recovery_grace || entry.releasing;
    if !owns_course(entry, target, flight.now)
        || !accepting
        || flight.now - entry.last_drop < DROP_INTERVAL_S
    {
        return;
    }
    let bomb = match first_bomb(ship) {
        Some(bomb) => bomb,
        None => return,
    };
    let prediction = predict_bomb(ship, target, bomb, &flight);
    let clearance = flight.own_radius + flight.target_radius + crate::bomber_math::HULL_MARGIN;
    let height_above_target = flight.target.1 - flight.position.1;
    let budget = (height_above_target - clearance).max(0.0);
    let needed_descent = MIN_RELEASE_DESCENT_SPEED
        .min(0.6 * Run::safe_descent(budget, flight.recovery_acceleration));
    if flight.now - entry.last_prediction_log >= 1.0 {
        entry.last_prediction_log = flight.now;
        log::info!(
            "ALLY bomber release check: c={ship:#x} height={:.1}/{:.0}, descent={:.1}/{:.1}, hit={}, miss={:.1}, tolerance={:.1}",
            height_above_target,
            RELEASE_HEIGHT,
            flight.velocity.1,
            needed_descent,
            prediction.as_ref().is_some_and(|prediction| prediction.crosses_target != 0),
            prediction.as_ref().map(|prediction| prediction.horizontal_miss).unwrap_or(f32::NAN),
            flight.target_radius * RELEASE_TOLERANCE_RADII,
        );
    }
    let first_bomb_of_salvo = entry.run.phase == Phase::Dive || recovery_grace;
    if first_bomb_of_salvo {
        // The native 10-second horizon reports a "hit" even from high altitude while almost
        // hovering, which released slow bombs far above the target. Require a real dive first:
        // low enough to arrive quickly, and actually moving down.
        let Some(prediction) = prediction else { return };
        let on_target = prediction.crosses_target != 0
            || prediction.horizontal_miss.abs() <= flight.target_radius * RELEASE_TOLERANCE_RADII;
        if height_above_target <= 0.0
            || height_above_target > RELEASE_HEIGHT
            || flight.velocity.1 < needed_descent
            || !on_target
        {
            return;
        }
        if entry.run.phase == Phase::Dive {
            entry.run.phase = Phase::Salvo;
        } else {
            // Already pulling out: keep the recovery course and just finish the salvo.
            entry.releasing = true;
        }
        log::info!("ALLY bomber: c={ship:#x} accurate release window acquired; starting 0.05s whole-payload salvo");
    }
    // SAFETY: the freshly borrowed part is still in this live ship's bomb vector. The native
    // detach routine handles parent links, physics and ownership; we never free/copy the part.
    let detach: unsafe extern "C" fn(usize) = std::mem::transmute(DETACH_BOMB);
    detach(bomb);
    entry.last_drop = flight.now;
    log::info!(
        "ALLY bomber: c={ship:#x} salvo dropped bomb={bomb:#x} at landed target={target:#x}, y={:.1}, descent={:.1}, t={:.3}",
        flight.position.1,
        flight.velocity.1,
        flight.now,
    );
    if first_bomb(ship).is_none() {
        begin_escape(entry, &flight);
    }
}

unsafe extern "C" fn on_release(ship: usize) {
    // Non-allied ships retain the original AI release, including enemy bombers.
    let allied = if battle::armed() {
        let Some(allies) = battle::try_allies() else {
            return;
        };
        allies.contains(&ship)
    } else {
        false
    };
    if !allied {
        let stock: unsafe extern "C" fn(usize) = std::mem::transmute(STOCK_RELEASE);
        stock(ship);
        return;
    }
    if !FAILED.load(Ordering::Relaxed) && catch_unwind(AssertUnwindSafe(|| release(ship))).is_err()
    {
        FAILED.store(true, Ordering::Relaxed);
        log::error!("hf_ally: panic in bomber release; ship bombers disabled");
    }
}

/// True when an active bomber course owns this ship's steering and must suppress its native
/// ship-missile launches. Fails closed when the run table cannot be locked.
///
/// # Safety
/// `ship` must be the live owner read from a launcher during the native launch call.
pub unsafe fn blocks_launch(ship: usize) -> bool {
    if FAILED.load(Ordering::Relaxed) || !battle::tracked(ship) {
        return false;
    }
    let Some((target, flight)) = flight(ship) else {
        return false;
    };
    let Ok(mut runs) = RUNS.try_lock() else {
        return true;
    };
    let Some(entry) = runs.iter_mut().find(|entry| entry.ship == ship) else {
        return false;
    };
    if !owns_course(entry, target, flight.now) {
        return false;
    }
    if !entry.missile_block_logged {
        entry.missile_block_logged = true;
        log::info!("ALLY bomber: c={ship:#x} missile launch blocked during bomber control");
    }
    true
}

/// Install only on the supported executable; the caller owns/commits the patch session.
/// # Safety
/// Must run during mod initialization, before AI callbacks, on Steam 1.163's fixed image.
pub unsafe fn install(session: &mut PatchSession) -> Result<(), String> {
    for (address, expected) in [
        (WAYPOINT_SITE, &WAYPOINT_BYTES[..]),
        (SPEED_SITE, &SPEED_BYTES[..]),
        (ENGINE_SITE, &ENGINE_BYTES[..]),
        (FINAL_ENGINE_SITE, &FINAL_ENGINE_BYTES[..]),
        (RELEASE_SITE, &RELEASE_BYTES[..]),
        (
            crate::native::SET_LINK,
            &crate::native::SET_LINK_PROLOGUE[..],
        ),
        (
            PREDICT_HIT,
            &[
                0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18, 0x48, 0x89, 0x48, 0x08,
            ][..],
        ),
        (
            FIRE_FLARE,
            &[0x48, 0x8b, 0xc4, 0x53, 0x55, 0x56, 0x57, 0x48][..],
        ),
        (
            WORLD_POSITION,
            &[0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x33, 0xc0][..],
        ),
        (
            DETACH_BOMB,
            &[0x48, 0x89, 0x5c, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec, 0x40][..],
        ),
        (
            STOCK_RELEASE,
            &[0x48, 0x8b, 0xc4, 0x55, 0x41, 0x57, 0x48, 0x8d, 0x68, 0xd8][..],
        ),
    ] {
        if std::slice::from_raw_parts(address as *const u8, expected.len()) != expected {
            log::warn!("hf_ally: bomber bytes differ at {address:#x}; ship bombers disabled");
            return Ok(());
        }
    }
    let mut setup = vec![0x48, 0xb8]; // mov rax, scratch; save the complete world waypoint
    setup.extend_from_slice(&(WAYPOINT.as_ptr() as u64).to_le_bytes());
    setup.extend_from_slice(&[
        0xf3, 0x44, 0x0f, 0x11, 0x08, 0xf3, 0x44, 0x0f, 0x11, 0x50, 0x04, 0x4c, 0x89, 0xf1,
    ]);
    let mut result = vec![0x48, 0xb8];
    result.extend_from_slice(&(WAYPOINT.as_ptr() as u64).to_le_bytes());
    result.extend_from_slice(&[
        0xf3, 0x44, 0x0f, 0x10, 0x08, 0xf3, 0x44, 0x0f, 0x10, 0x50, 0x04,
    ]);
    // The original X store is before this hook. Repeat it after hf_ai/our callback so the
    // native waypoint and the registers used by the rest of the tick describe the same course.
    result.extend_from_slice(&STORE_WAYPOINT_X);
    let mut steer = Trampoline::new();
    steer.preserved_call(on_steer as *const (), &setup, &result, ReturnType::None);
    steer.bytes(&WAYPOINT_BYTES);
    steer.absolute_jump(WAYPOINT_SITE + WAYPOINT_BYTES.len());
    session
        .detour_trampoline(WAYPOINT_SITE, WAYPOINT_BYTES.len(), steer)
        .map_err(|error| error.to_string())?;
    let mut speed_setup = vec![0x48, 0xb8]; // mov rax,scratch; movss [rax],xmm12; mov rcx,r14
    speed_setup.extend_from_slice(&(std::ptr::addr_of!(WANTED_SPEED) as u64).to_le_bytes());
    speed_setup.extend_from_slice(&[0xf3, 0x44, 0x0f, 0x11, 0x20, 0x4c, 0x89, 0xf1]);
    let mut speed_result = vec![0x48, 0xb8]; // mov rax,scratch; movss xmm12,[rax]
    speed_result.extend_from_slice(&(std::ptr::addr_of!(WANTED_SPEED) as u64).to_le_bytes());
    speed_result.extend_from_slice(&[0xf3, 0x44, 0x0f, 0x10, 0x20]);
    let mut speed = Trampoline::new();
    speed.bytes(&SPEED_BYTES);
    speed.preserved_call(
        on_speed as *const (),
        &speed_setup,
        &speed_result,
        ReturnType::None,
    );
    speed.absolute_jump(SPEED_SITE + SPEED_BYTES.len());
    session
        .detour_trampoline(SPEED_SITE, SPEED_BYTES.len(), speed)
        .map_err(|error| error.to_string())?;
    let mut engines = Trampoline::new();
    engines.bytes(&ENGINE_BYTES);
    engines.preserved_call(
        on_engines as *const (),
        &[0x4c, 0x89, 0xf1, 0x48, 0x8d, 0x55, 0x28], // mov rcx,r14; lea rdx,[rbp+0x28]
        &[],
        ReturnType::None,
    );
    engines.absolute_jump(ENGINE_SITE + ENGINE_BYTES.len());
    session
        .detour_trampoline(ENGINE_SITE, ENGINE_BYTES.len(), engines)
        .map_err(|error| error.to_string())?;
    let mut final_engines = Trampoline::new();
    final_engines.preserved_call(
        on_final_engines as *const (),
        &[0x4c, 0x89, 0xf1, 0x48, 0x8d, 0x55, 0x28], // mov rcx,r14; lea rdx,[rbp+0x28]
        &[],
        ReturnType::None,
    );
    final_engines.bytes(&FINAL_ENGINE_BYTES);
    final_engines.absolute_jump(FINAL_ENGINE_SITE + FINAL_ENGINE_BYTES.len());
    session
        .detour_trampoline(FINAL_ENGINE_SITE, FINAL_ENGINE_BYTES.len(), final_engines)
        .map_err(|error| error.to_string())?;
    let mut release = Trampoline::new();
    release.preserved_call(on_release as *const (), &[], &[], ReturnType::None);
    release.absolute_jump(RELEASE_SITE + RELEASE_BYTES.len());
    session
        .detour_trampoline(RELEASE_SITE, RELEASE_BYTES.len(), release)
        .map_err(|error| error.to_string())?;
    log::info!("hf_ally: ally ship bombers enabled (matched swivel/thrust controls, native aimed 0.05s salvo, no speed/height/timer gate, forward pull-up; landed non-prize targets only)");
    Ok(())
}
