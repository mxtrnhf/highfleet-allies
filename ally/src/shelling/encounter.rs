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
    sound,
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
// Native full-screen BLACK overlay singleton (Gui_CreateBlackOverlay, 0x1401ca120).
const BLACKOUT_OVERLAY: usize = 0x1_43a1_3b08;
const BLACKOUT_STATE: usize = 0x3cc;
const BLACKOUT_RATE: usize = 0x3d0;
const BLACKOUT_ALPHA: usize = 0x8f8;
const BATTLE_RENDER: usize = 0x1_4030_50a0;
const BATTLE_RENDER_CALL: usize = 0x1_4030_0d7d;
const COMBAT_EVENTS: usize = 0x1_4029_a7d0;
const COMBAT_EVENTS_CALL: usize = 0x1_4030_93fa;
const MAX_DEFENDERS: usize = 4; // Native setup limit.
const MAX_ROUNDS_PER_DEFENDER: u32 = 3;
const MAX_GROUND_ROUNDS: u32 = 30;
const MAX_IN_FLIGHT: usize = 6;
const WIND_ONLY_SECONDS: f64 = 2.6;
const AFTERMATH_SECONDS: f64 = 1.2;
// Tactical raid tracer descent duration (seconds from top of sky to impact).
// Increase FLIGHT_SECONDS_BASE for slower, more readable descending shells; decrease for faster shells.
const FLIGHT_SECONDS_BASE: f64 = 0.58;
const FLIGHT_JITTER_SECONDS: f64 = 0.08;
const WHOOSH_LEAD_SECONDS: f64 = 0.38;

#[derive(Clone, Copy)]
pub(super) struct Bombardment {
    pub hits: u32,
    pub rounds: u32,
    pub seed: u64,
    /// Horizontal incoming side from the firing fleet's strategic bearing (-1.0 = left, +1.0 = right).
    pub incoming_direction: f32,
}

impl Bombardment {
    fn ground_rounds(self) -> u32 {
        let misses = self.rounds.saturating_sub(self.hits);
        // Compress thousands of real misses into a short scene without making a single gun
        // and a full battery look alike.
        let volume_bands = (1 + misses / 100).ilog2();
        let hull_share = self.hits.min(MAX_ROUNDS_PER_DEFENDER * 4);
        // The dirt is the spectacle: show at least twice as many misses as hull hits.
        let representative = (2 + 3 * volume_bands).max(2 * hull_share);
        representative.min(MAX_GROUND_ROUNDS).min(misses)
    }
}

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
    lands_at: f64,
    cue: u32,
    whoosh_pending: bool,
}

#[derive(Clone, Copy)]
enum Cadence {
    /// Steady multi-gun battery roll with an even beat.
    Rhythmic { left: u8, beat: f64 },
    /// Irregular scattered cluster with uneven spacing.
    Irregular { left: u8 },
}

struct Raid {
    hits: u32,
    incoming_direction: f32,
    opening_defenders: [i64; MAX_DEFENDERS],
    next_hit: u32,
    hit_impacts: u32,
    ground_impacts: u32,
    opening_count: u8,
    opening_started: bool,
    cadence: Cadence,
    ground_left: u32,
    intensity: f32,
    random: u64,
    next_at: f64,
    time: f64,
    finish_at: Option<f64>,
    flights: Vec<Flight>,
    frame: Option<Frame>,
    destroyed: usize,
}

impl Raid {
    fn roll(&mut self) -> u32 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        (self.random >> 32) as u32
    }

    fn sample(&mut self) -> f64 {
        f64::from(self.roll()) / 4_294_967_296.0
    }

    fn space_next_launch(&mut self, now: f64) {
        let gap = match self.cadence {
            Cadence::Rhythmic { left, beat } if left > 0 => {
                self.cadence = Cadence::Rhythmic {
                    left: left - 1,
                    beat,
                };
                // Tiny micro-variation keeps a rhythmic battery roll feeling mechanical, not robotic.
                (beat + (self.sample() - 0.5) * 0.016).max(0.08)
            }
            Cadence::Irregular { left } if left > 0 => {
                self.cadence = Cadence::Irregular { left: left - 1 };
                0.05 + 0.22 * self.sample()
            }
            _ => {
                // Cluster finished: pick the next pattern (rhythmic roll, irregular burst, or stray)
                // after a real lull.
                let pattern = self.sample();
                let scale = f64::from(self.intensity);
                self.cadence = if pattern < 0.45 {
                    let count = 2 + (self.sample() * (2.5 + 2.5 * scale)) as u8;
                    let beat = 0.13 + 0.09 * self.sample();
                    Cadence::Rhythmic { left: count, beat }
                } else if pattern < 0.85 {
                    let count = 1 + (self.sample() * (2.0 + 2.0 * scale)) as u8;
                    Cadence::Irregular { left: count }
                } else {
                    Cadence::Irregular { left: 0 }
                };
                0.55 + 1.25 * self.sample().powi(2)
            }
        };
        self.next_at = now + gap;
    }
}

enum State {
    Idle,
    Loading(Bombardment),
    Active(Box<Raid>),
}

static RAID: Mutex<State> = Mutex::new(State::Idle);

/// Stage the same native side A links as a map encounter, leaving side B empty.
/// Never retain a Rust list or raw fleet pointer across the level transition.
///
/// # Safety
/// Game-thread map input callback with a live campaign, no pending transition,
/// and borrowed fleets from the current profile.
pub(super) unsafe fn start(fleets: &[usize], volume: Bombardment) -> bool {
    if fleets.is_empty() || volume.hits == 0 || read::<u8>(SCENE_FLAGS[2]) != 0 {
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
        *raid = State::Loading(volume);
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
        "hf_ally: shelling raid loading: {} effective map hits from {} rounds; {} representative ground rounds; {} defender fleets",
        volume.hits,
        volume.rounds,
        volume.ground_rounds(),
        fleets.len(),
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
        if let State::Loading(volume) = *state {
            let now = read::<f64>(CLOCK);
            let ground_left = volume.ground_rounds();
            // Allocate raid-local storage once, at native setup, not in the render callback.
            *state = State::Active(Box::new(Raid {
                hits: volume.hits,
                incoming_direction: if volume.incoming_direction < 0.0 {
                    -1.0
                } else {
                    1.0
                },
                opening_defenders: [0; MAX_DEFENDERS],
                next_hit: 0,
                hit_impacts: 0,
                ground_impacts: 0,
                opening_count: 0,
                opening_started: false,
                cadence: Cadence::Irregular { left: 0 },
                ground_left,
                intensity: ground_left as f32 / MAX_GROUND_ROUNDS as f32,
                random: volume.seed.rotate_right(23).max(1),
                next_at: f64::INFINITY,
                time: now,
                finish_at: None,
                flights: Vec::with_capacity(MAX_IN_FLIGHT),
                frame: None,
                destroyed: 0,
            }));
            // SAFETY: the native level setup has initialized its full-screen HUD overlay.
            hold_blackout(0.3, 1.0);
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

/// Keep the stock full-screen BLACK overlay opaque while the level finishes loading.
///
/// # Safety
/// Called on the game thread after the native HUD overlay is initialized. Only fixed scalar
/// fields on the game-owned singleton are written; no pointer is retained or dropped.
unsafe fn hold_blackout(rate: f32, alpha: f32) {
    let overlay = read::<usize>(BLACKOUT_OVERLAY);
    if overlay == 0 || !readable_span(overlay, BLACKOUT_ALPHA + size_of::<f32>()) {
        return;
    }
    ((overlay + BLACKOUT_STATE) as *mut i32).write_unaligned(3);
    ((overlay + BLACKOUT_RATE) as *mut f32).write_unaligned(rate);
    ((overlay + BLACKOUT_ALPHA) as *mut f32).write_unaligned(alpha);
}

/// Let the existing native blackout overlay clear at the stock fade rate.
///
/// # Safety
/// Same game-thread singleton contract as [`hold_blackout`].
unsafe fn fade_blackout_out(rate: f32) {
    let overlay = read::<usize>(BLACKOUT_OVERLAY);
    if overlay == 0 || !readable_span(overlay, BLACKOUT_STATE + size_of::<i32>()) {
        return;
    }
    // `Gui_BlackoutOverlay_BeginClear` arms state 0 with the incoming rate, then the GUI fades
    // it to state 1. Writing both fields mirrors that stock transition.
    ((overlay + BLACKOUT_STATE) as *mut i32).write_unaligned(0);
    ((overlay + BLACKOUT_RATE) as *mut f32).write_unaligned(rate);
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
    let play_wind = !raid.opening_started;
    if play_wind {
        raid.opening_started = true;
        raid.next_at = now + WIND_ONLY_SECONDS;
    }
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
    let mut landed = [None; MAX_IN_FLIGHT];
    let mut slots = landed.iter_mut();
    let mut whooshes = [None; MAX_IN_FLIGHT];
    let mut whoosh_slots = whooshes.iter_mut();
    raid.flights.retain_mut(|flight| {
        if let Some([x, y]) = flight
            .ship
            .and_then(|target| ship_by_id(&ships, target.ship_id))
            .and_then(|ship| center(ship))
        {
            if let Some(target) = flight.ship {
                flight.point = [x + target.contact_offset[0], y + target.contact_offset[1]];
            }
        }
        if flight.whoosh_pending && flight.lands_at - now <= WHOOSH_LEAD_SECONDS {
            flight.whoosh_pending = false;
            if let Some(slot) = whoosh_slots.next() {
                *slot = Some(flight.cue);
            }
        }
        if now < flight.lands_at {
            return true;
        }
        if let Some(slot) = slots.next() {
            *slot = Some(*flight);
        }
        false
    });
    if now >= raid.next_at && raid.flights.len() < MAX_IN_FLIGHT {
        schedule_round(raid, &ships, defender_count, total, now);
    }
    if raid.next_hit >= total && raid.ground_left == 0 && raid.flights.is_empty() {
        raid.finish_at.get_or_insert(now + AFTERMATH_SECONDS);
    }
    let continuing = raid.finish_at.is_none_or(|finish| now < finish);
    let intensity = raid.intensity;
    drop(state);

    if play_wind {
        // SAFETY: the native raid scene is live on its game thread. Sound uses the GUI root.
        sound::shelling_wind();
        fade_blackout_out(0.7);
    }
    for cue in whooshes.into_iter().flatten() {
        // SAFETY: game-thread raid tick outside the RAID lock; GUI root is live.
        sound::shelling_whoosh(cue);
    }
    let mut hits = 0;
    let mut misses = 0;
    let mut destroyed = 0;
    for flight in landed.into_iter().flatten() {
        let mut point = flight.point;
        let ground = flight.ship.is_none();
        if let Some(target) = flight.ship {
            let Some(ship) = ship_by_id(&ships, target.ship_id)
                .filter(|&ship| is_ship(ship) && crate::native::live_ship(ship))
            else {
                continue;
            };
            let Some([x, y]) = center(ship) else {
                continue;
            };
            let aim = [x + target.aim_offset[0], y + target.aim_offset[1]];
            let Some(report) = strike::apply_to_ship(ship, aim, flight.direction) else {
                continue;
            };
            point = report.point;
            hits += 1;
            destroyed += report.destroyed;
            log::info!(
                "hf_ally: shelling raid impact: ship={ship:#x}, removed={}, hull {:.0} -> {:.0}",
                report.destroyed,
                report.health_before,
                report.health_after
            );
        } else {
            if !presentation::clear_of_hulls(
                point,
                ships
                    .iter()
                    .copied()
                    .filter(|&ship| is_ship(ship) && crate::native::live_ship(ship))
                    .filter_map(|ship| hull(ship)),
            ) {
                continue;
            }
            misses += 1;
        }
        if point.iter().all(|coordinate| coordinate.is_finite()) {
            presentation::impact(point, ground, intensity, flight.cue);
        }
    }
    if hits != 0 || misses != 0 {
        if let Ok(mut state) = RAID.lock() {
            if let State::Active(raid) = &mut *state {
                raid.hit_impacts += hits;
                raid.ground_impacts += misses;
                raid.destroyed += destroyed;
            }
        }
    }
    u8::from(continuing)
}

/// Mix a bounded sample of misses into the existing hit allocation. A lost defender's allotted
/// hits are skipped; misses never borrow or consume another ship's allocation.
unsafe fn schedule_round(
    raid: &mut Raid,
    ships: &[usize],
    defender_count: usize,
    total: u32,
    now: f64,
) {
    while raid.next_hit < total || raid.ground_left != 0 {
        let hits_left = total - raid.next_hit;
        let ground_ratio = f64::from(raid.ground_left) / f64::from(raid.ground_left + hits_left);
        let ground = raid.sample() < ground_ratio;
        let direction = raid.incoming_direction;
        let (ship, point) = if ground {
            raid.ground_left -= 1;
            let scatter = raid.sample() as f32;
            let Some(point) = raid.frame.and_then(|frame| {
                frame.miss_point(scatter, ships.iter().filter_map(|&ship| hull(ship)))
            }) else {
                continue;
            };
            (None, point)
        } else {
            let order = raid.next_hit;
            raid.next_hit += 1;
            let Some(&id) = raid.opening_defenders.get(order as usize % defender_count) else {
                continue;
            };
            let Some(ship) = ship_by_id(ships, id) else {
                continue;
            };
            let Some([x, y]) = center(ship) else {
                continue;
            };
            let Some((aim, point)) = strike::aim(ship, direction) else {
                continue;
            };
            (
                Some(ShipTarget {
                    ship_id: id,
                    aim_offset: [aim[0] - x, aim[1] - y],
                    contact_offset: [point[0] - x, point[1] - y],
                }),
                point,
            )
        };
        let lands_at = now + FLIGHT_SECONDS_BASE + (raid.sample() - 0.5) * FLIGHT_JITTER_SECONDS;
        let cue = raid.roll();
        // Occasional subtle incoming whoosh (~1 in 4 rounds) right before terminal impact.
        let whoosh_pending = (cue & 3) == 0;
        raid.flights.push(Flight {
            ship,
            point,
            direction,
            fired_at: now,
            lands_at,
            cue,
            whoosh_pending,
        });
        raid.space_next_launch(now);
        return;
    }
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
    let time = raid.time;
    let mut slots = raid.flights.iter().copied();
    let flights: [Option<Flight>; MAX_IN_FLIGHT] = std::array::from_fn(|_| slots.next());
    drop(state);
    let Some(sky) = presentation::SkyEntry::capture() else {
        return;
    };
    for flight in flights.into_iter().flatten() {
        let fraction = ((time - flight.fired_at) / (flight.lands_at - flight.fired_at)) as f32;
        if fraction.is_finite() && flight.point.iter().all(|n| n.is_finite()) {
            sky.tracer(flight.point, flight.direction, fraction);
        }
    }
}
