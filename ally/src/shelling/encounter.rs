//! Short spectator raid using Steam 1.163's SCR_MISSILE defender lifecycle.
//! Unlike ordinary Combat, this returns surviving defenders to the campaign immediately.
//! Native contracts and otherwise unmodified state transitions: docs/strategic-shelling.md.

use std::{
    mem::{offset_of, size_of},
    sync::Mutex,
};

use patchy::{Condition, PatchSession, ReturnType, Trampoline};

use crate::{
    diag::readable_span,
    native::{read, roster, CLOCK, SCENE_FLAGS},
    native_layout::{Creature, Link, Node},
};

use super::{
    is_ship,
    presentation::{self, Frame, Hull, Point},
    strike,
};

const SIDE_A: usize = 0x1_43a1_58e0;
const SIDE_B: usize = 0x1_43a1_59c0;
const LINK_CONSTRUCT: usize = 0x1_4019_4bd0;
const LINK_COPY: usize = 0x1_4019_4b80;
const LINK_UNLINK: usize = 0x1_4019_4d20;
const LINK_VEC_CLEAR: usize = 0x1_400c_2400;
const LINK_VEC_GROW: usize = 0x1_4000_4b00;
const LOAD_LEVEL: usize = 0x1_4025_afd0;
const MISSILE_TICK: usize = 0x1_402d_4330;
const BATTLE_RENDER: usize = 0x1_4030_50a0;
const BATTLE_RENDER_CALL: usize = 0x1_4030_0d7d;
const COMBAT_EVENTS: usize = 0x1_4029_a7d0;
const COMBAT_EVENTS_CALL: usize = 0x1_4030_93fa;
const MAX_DEFENDERS: usize = 4; // Native setup limit.
const MAX_ROUNDS_PER_DEFENDER: u32 = 3;
const HITS_PER_COSMETIC_MISS: u32 = 3;
const OPENING_SECONDS: f64 = 0.8;
const FLIGHT_SECONDS: f64 = 0.32;
const IMPACT_GAP_SECONDS: f64 = 0.36;
const AFTERMATH_SECONDS: f64 = 1.2;

#[derive(Clone, Copy)]
struct ShipTarget {
    ship_id: i64,
    aim_offset: Point,
    contact_offset: Point,
}

#[derive(Clone, Copy)]
struct Flight {
    ship: Option<ShipTarget>,
    direction: f32,
    point: Point,
    fired_at: f64,
}

struct Raid {
    hits: u32,
    opening_defenders: [i64; MAX_DEFENDERS],
    next_hit: u32,
    hit_impacts: u32,
    ground_impacts: u32,
    opening_count: u8,
    miss_due: bool,
    next_at: f64,
    time: f64,
    finish_at: Option<f64>,
    flight: Option<Flight>,
    frame: Option<Frame>,
    destroyed: usize,
}

enum State {
    Idle,
    Loading(u32),
    Active(Raid),
}

static RAID: Mutex<State> = Mutex::new(State::Idle);

/// Stage the same native side A links as a map encounter, leaving side B empty.
/// Never retain a Rust list or raw fleet pointer across the level transition.
///
/// # Safety
/// Game-thread map input callback with a live campaign, no pending transition,
/// and borrowed fleets from the current profile.
pub(super) unsafe fn start(fleets: &[usize], hits: u32) -> bool {
    if fleets.is_empty() || hits == 0 || read::<u8>(SCENE_FLAGS[2]) != 0 {
        return false;
    }
    for vector in [SIDE_A, SIDE_B] {
        let [begin, end, capacity]: [usize; 3] = read(vector);
        if begin > end
            || end > capacity
            || !(capacity - begin).is_multiple_of(size_of::<Link>())
            || capacity - begin > 4096 * size_of::<Link>()
            || (capacity > begin && !readable_span(begin, capacity - begin))
        {
            return false;
        }
    }
    {
        let Ok(raid) = RAID.try_lock() else {
            return false;
        };
        if !matches!(*raid, State::Idle) {
            return false;
        }
    }
    if let Err(reason) = strike::prepare_garrisons(fleets) {
        log::warn!("hf_ally: shelling raid not started: {reason}");
        return false;
    }
    {
        let Ok(mut raid) = RAID.lock() else {
            return false;
        };
        if !matches!(*raid, State::Idle) {
            return false;
        }
        *raid = State::Loading(hits);
    }
    let clear: unsafe extern "C" fn(usize) = std::mem::transmute(LINK_VEC_CLEAR);
    let load: unsafe extern "C" fn(*const i8, u8, u8) = std::mem::transmute(LOAD_LEVEL);
    // SAFETY: the game owns both vectors and all linked objects. No other native
    // encounter is pending, and this is the same staging order as UpdateStrategicFleet.
    clear(SIDE_A);
    clear(SIDE_B);
    for &fleet in fleets {
        push_link(SIDE_A, fleet);
    }
    log::info!(
        "hf_ally: shelling raid loading: {hits} effective map hits; {} defender fleets",
        fleets.len()
    );
    load(c"Missile.seria".as_ptr(), 0, 0);
    true
}

/// Uses native construction, copying, growth, and unlinking in the same order as
/// the map's own engagement builder. The temporary Link is native scratch storage.
unsafe fn push_link(vector: usize, node: usize) {
    let mut scratch = [0usize; 3];
    let construct: unsafe extern "C" fn(*mut usize, usize) -> usize =
        std::mem::transmute(LINK_CONSTRUCT);
    let copy: unsafe extern "C" fn(usize, *const usize) -> usize = std::mem::transmute(LINK_COPY);
    let grow: unsafe extern "C" fn(usize, usize, *const usize) -> usize =
        std::mem::transmute(LINK_VEC_GROW);
    let unlink: unsafe extern "C" fn(*mut usize) = std::mem::transmute(LINK_UNLINK);
    construct(scratch.as_mut_ptr(), node);
    let end = read::<usize>(vector + size_of::<usize>());
    let capacity = read::<usize>(vector + 2 * size_of::<usize>());
    if end == capacity {
        grow(vector, end, scratch.as_ptr());
    } else {
        copy(end, scratch.as_ptr());
        ((vector + size_of::<usize>()) as *mut usize).write_unaligned(end + size_of::<Link>());
    }
    unlink(scratch.as_mut_ptr());
}

pub(super) unsafe fn install(session: &mut PatchSession) -> Result<(), String> {
    // B is empty only for our strike. Skip the native B[0] missile creation/
    // count reset and continue at the common setup epilogue (including HUD setup).
    let mut spawn = Trampoline::new();
    let stock = spawn.new_label();
    spawn.preserved_call(is_loading as *const (), &[], &[], ReturnType::Rax);
    spawn.bytes(&[0x84, 0xc0]); // test al, al
    spawn.jump_if(Condition::Equal, stock);
    spawn.absolute_jump(0x1_402d_4ea5);
    spawn.bind(stock).map_err(|err| err.to_string())?;
    // Replay RIP-relative MOV RAX,[SIDE_B::begin] without borrowing a scratch register.
    spawn.bytes(&[0x48, 0xb8]);
    spawn.bytes(&SIDE_B.to_le_bytes());
    spawn.bytes(&[0x48, 0x8b, 0x00]);
    spawn.absolute_jump(0x1_402d_4cf0);
    session
        .detour_trampoline(0x1_402d_4ce9, 7, spawn)
        .map_err(|err| err.to_string())?;

    // Native teardown has already extracted the defenders before it reads B[0].
    // Skip only the ordnance restitution/positioning block, not the fleet aftermath.
    let mut extract = Trampoline::new();
    let stock = extract.new_label();
    extract.preserved_call(is_raid as *const (), &[], &[], ReturnType::Rax);
    extract.bytes(&[0x84, 0xc0]);
    extract.jump_if(Condition::Equal, stock);
    extract.bytes(&[0x48, 0x8b, 0x74, 0x24, 0x48]); // Restore RSI from [rsp+0x48].
    extract.absolute_jump(0x1_402d_509d);
    extract.bind(stock).map_err(|err| err.to_string())?;
    extract.bytes(&[0x48, 0xb8]);
    extract.bytes(&SIDE_B.to_le_bytes());
    extract.bytes(&[0x48, 0x8b, 0x00]);
    extract.absolute_jump(0x1_402d_4f9a);
    session
        .detour_trampoline(0x1_402d_4f93, 7, extract)
        .map_err(|err| err.to_string())?;

    // Shipworks owns the setup/teardown calls themselves. Use the next
    // instruction in each case so both features retain the same native calls.
    let mut setup = Trampoline::new();
    let normal = setup.new_label();
    setup.preserved_call(after_setup as *const (), &[], &[], ReturnType::Rax);
    setup.bytes(&[0x84, 0xc0]);
    setup.jump_if(Condition::Equal, normal);
    // No player ship in the spectator raid; skip the no-control tutorial dialog.
    setup.absolute_jump(0x1_402c_4e78);
    setup.bind(normal).map_err(|err| err.to_string())?;
    setup.absolute_call(crate::native::GET_PROFILE as *const ());
    setup.absolute_jump(0x1_402c_4e06);
    session
        .detour_trampoline(0x1_402c_4e01, 5, setup)
        .map_err(|err| err.to_string())?;

    let mut raid_tick = Trampoline::new();
    raid_tick.preserved_call(tick as *const (), &[], &[], ReturnType::Rax);
    raid_tick.absolute_jump(0x1_402c_5059);
    session
        .detour_trampoline(0x1_402c_5054, 5, raid_tick)
        .map_err(|err| err.to_string())?;

    let mut finish = Trampoline::new();
    finish.preserved_call(after_teardown as *const (), &[], &[], ReturnType::None);
    finish.bytes(&[0x45, 0x33, 0xc0, 0x33, 0xd2]); // xor r8d,r8d; xor edx,edx
    finish.absolute_jump(0x1_402c_50fb);
    session
        .detour_trampoline(0x1_402c_50f6, 5, finish)
        .map_err(|err| err.to_string())?;

    // Use the battle renderer, not the player/ally HUD branch (which can be absent in a raid).
    let mut camera = Trampoline::new();
    camera.absolute_call(render_battle as *const ());
    camera.relative_jump(BATTLE_RENDER_CALL + 5);
    session
        .detour_trampoline(BATTLE_RENDER_CALL, 5, camera)
        .map_err(|err| err.to_string())?;

    // This call precedes the battle renderer's stock overlay flush at 0x140309408.
    let mut overlay = Trampoline::new();
    overlay.absolute_call(COMBAT_EVENTS as *const ());
    overlay.preserved_call(draw_tracer as *const (), &[], &[], ReturnType::None);
    overlay.relative_jump(COMBAT_EVENTS_CALL + 5);
    session
        .detour_trampoline(COMBAT_EVENTS_CALL, 5, overlay)
        .map_err(|err| err.to_string())?;

    presentation::install(session)
}

extern "C" fn is_loading() -> bool {
    RAID.lock()
        .is_ok_and(|raid| matches!(*raid, State::Loading(_)))
}

extern "C" fn is_raid() -> bool {
    RAID.lock().is_ok_and(|raid| !matches!(*raid, State::Idle))
}

unsafe extern "C" fn after_setup() -> u8 {
    if let Ok(mut state) = RAID.lock() {
        if let State::Loading(hits) = *state {
            let now = read::<f64>(CLOCK);
            *state = State::Active(Raid {
                hits,
                opening_defenders: [0; MAX_DEFENDERS],
                next_hit: 0,
                hit_impacts: 0,
                ground_impacts: 0,
                opening_count: 0,
                miss_due: false,
                next_at: now + OPENING_SECONDS,
                time: now,
                finish_at: None,
                flight: None,
                frame: None,
                destroyed: 0,
            });
            return 1;
        }
    }
    0
}

unsafe fn defenders() -> Vec<usize> {
    roster()
        .into_iter()
        .take(MAX_DEFENDERS)
        .filter(|&ship| {
            is_ship(ship)
                && crate::native::live_ship(ship)
                && strike::is_expanded(ship)
                && read::<f32>(ship + offset_of!(Creature, side)) < 0.0
        })
        .collect()
}

unsafe fn center(ship: usize) -> Option<Point> {
    let position: Point = read(ship + offset_of!(Creature, bounds_center_x));
    position.iter().all(|n| n.is_finite()).then_some(position)
}

unsafe fn hull(ship: usize) -> Option<Hull> {
    let radius: f32 = read(ship + offset_of!(Creature, bounding_radius));
    let center = center(ship)?;
    (radius.is_finite() && radius > 0.0).then_some(Hull { center, radius })
}

unsafe fn ship_by_id(ships: &[usize], id: i64) -> Option<usize> {
    ships
        .iter()
        .copied()
        .find(|&ship| read::<i64>(ship + offset_of!(Node, id)) == id)
}

unsafe extern "C" fn tick() -> u8 {
    let now = read::<f64>(CLOCK);
    let active = RAID
        .lock()
        .is_ok_and(|state| matches!(*state, State::Active(_)));
    if !active {
        let stock: unsafe extern "C" fn() -> u8 = std::mem::transmute(MISSILE_TICK);
        return stock();
    }
    if !now.is_finite() {
        return 0;
    }
    let ships = defenders();
    let Ok(mut state) = RAID.lock() else {
        return 0;
    };
    let State::Active(raid) = &mut *state else {
        return 0;
    };
    raid.time = now;
    if raid.opening_count == 0 {
        raid.opening_count = ships.len() as u8;
        for (slot, &ship) in raid.opening_defenders.iter_mut().zip(&ships) {
            *slot = read(ship + offset_of!(Node, id));
        }
    }
    if raid.frame.is_none() && !ships.is_empty() {
        raid.frame = Frame::capture(ships.iter().filter_map(|&ship| hull(ship)));
    }
    let defender_count = usize::from(raid.opening_count);
    let total = raid
        .hits
        .min(defender_count as u32 * MAX_ROUNDS_PER_DEFENDER);
    let mut impact = None;
    if let Some(flight) = raid.flight.as_mut() {
        let live_target = flight.ship.and_then(|target| {
            let ship = ship_by_id(&ships, target.ship_id)?;
            let [x, y] = center(ship)?;
            flight.point = [x + target.contact_offset[0], y + target.contact_offset[1]];
            Some((ship, [x + target.aim_offset[0], y + target.aim_offset[1]]))
        });
        if now >= flight.fired_at + FLIGHT_SECONDS {
            if flight.ship.is_none() {
                raid.ground_impacts += 1;
                impact = Some((flight.point, None, raid.hit_impacts + raid.ground_impacts));
            } else if let Some((ship, aim)) = live_target {
                raid.hit_impacts += 1;
                raid.miss_due = raid.hit_impacts % HITS_PER_COSMETIC_MISS == 1;
                impact = Some((
                    flight.point,
                    Some((ship, aim, flight.direction)),
                    raid.hit_impacts + raid.ground_impacts,
                ));
            }
            raid.flight = None;
            raid.next_at = now + IMPACT_GAP_SECONDS;
        }
    }
    if raid.flight.is_none() && now >= raid.next_at {
        if std::mem::take(&mut raid.miss_due) {
            if let Some(point) = raid.frame.and_then(|frame| {
                frame.miss_point(
                    raid.ground_impacts,
                    ships.iter().filter_map(|&ship| hull(ship)),
                )
            }) {
                raid.flight = Some(Flight {
                    ship: None,
                    direction: if raid.ground_impacts % 2 == 0 {
                        -1.0
                    } else {
                        1.0
                    },
                    point,
                    fired_at: now,
                });
            }
        }
        // Round-robin over opening slots. A sunk defender's rounds are not moved onto survivors.
        while raid.flight.is_none() && raid.next_hit < total {
            let order = raid.next_hit;
            raid.next_hit += 1;
            let slot = order as usize % defender_count;
            let Some(&id) = raid.opening_defenders.get(slot) else {
                continue;
            };
            let Some(ship) = ship_by_id(&ships, id) else {
                continue;
            };
            let Some([x, y]) = center(ship) else {
                continue;
            };
            let direction = if order % 2 == 0 { 1.0 } else { -1.0 };
            let (aim, point) = strike::aim(ship, direction).unwrap_or(([x, y], [x, y]));
            raid.flight = Some(Flight {
                ship: Some(ShipTarget {
                    ship_id: id,
                    aim_offset: [aim[0] - x, aim[1] - y],
                    contact_offset: [point[0] - x, point[1] - y],
                }),
                direction,
                point,
                fired_at: now,
            });
        }
    }
    if raid.next_hit >= total && raid.flight.is_none() && !raid.miss_due {
        raid.finish_at.get_or_insert(now + AFTERMATH_SECONDS);
    }
    let continuing = raid.finish_at.is_none_or(|finish| now < finish);
    drop(state);

    if let Some((mut point, hit, ordinal)) =
        impact.filter(|(point, _, _)| point.iter().all(|coordinate| coordinate.is_finite()))
    {
        if let Some((ship, aim, direction)) = hit {
            if let Some(report) = strike::apply_to_ship(ship, aim, direction) {
                point = report.point;
                if let Ok(mut state) = RAID.lock() {
                    if let State::Active(raid) = &mut *state {
                        raid.destroyed += report.destroyed;
                    }
                }
                log::info!(
                    "hf_ally: shelling raid impact: ship={ship:#x}, removed={}, hull {:.0} -> {:.0}",
                    report.destroyed,
                    report.health_before,
                    report.health_after
                );
            }
        }
        presentation::impact(point, hit.is_none(), ordinal);
    }
    u8::from(continuing)
}

extern "C" fn after_teardown() {
    let raid = RAID
        .lock()
        .ok()
        .map(|mut state| std::mem::replace(&mut *state, State::Idle));
    if let Some(State::Active(raid)) = raid {
        log::info!(
            "hf_ally: shelling raid native return completed: {} hull impacts, {} ground misses, {} parts removed",
            raid.hit_impacts,
            raid.ground_impacts,
            raid.destroyed
        );
    }
}

unsafe extern "C" fn render_battle(first: usize, second: usize, third: usize) {
    let frame = RAID.lock().ok().and_then(|state| match &*state {
        State::Active(raid) => raid.frame,
        _ => None,
    });
    let stock: unsafe extern "C" fn(usize, usize, usize) = std::mem::transmute(BATTLE_RENDER);
    let draw = || stock(first, second, third);
    if let Some(frame) = frame {
        frame.render(draw);
    } else {
        draw();
    }
}

/// Snapshot only scalar flight data. A spectator raid need not have a player HUD or ship.
unsafe extern "C" fn draw_tracer() {
    let Ok(state) = RAID.lock() else {
        return;
    };
    let State::Active(raid) = &*state else {
        return;
    };
    let Some(flight) = raid.flight else {
        return;
    };
    let fraction = ((raid.time - flight.fired_at) / FLIGHT_SECONDS) as f32;
    drop(state);
    if fraction.is_finite() && flight.point.iter().all(|n| n.is_finite()) {
        presentation::tracer(flight.point, flight.direction, fraction);
    }
}
