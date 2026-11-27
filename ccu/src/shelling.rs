//! Strategic shelling: a preview, a mobile barrage and garrison damage.
//! Native evidence and the damage contract: `docs/strategic-shelling.md`.
//! A barrage consumes no inventory and does not damage city infrastructure.

mod encounter;
mod estimate;
mod presentation;
mod strike;
mod visual;

use std::{
    ffi::CStr,
    mem::{offset_of, size_of},
    ops::Range,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
        Mutex,
    },
    time::{Duration as WallDuration, Instant},
};

use crate::patchy::{PatchSession, ReturnType, Trampoline};
use highfleet::general::SentinelListNode;

use crate::{
    gui,
    memory::{rd, readable_span},
    native::{
        plausible, read, GetProfileFn, ACTIVE_DIALOG, GET_PROFILE, IN_COMBAT, OFF_ACTIVE,
        SCENE_FLAGS, SHIP_LIST_LIMIT,
    },
    native_layout::{Body, CVec, Creature, Gui, Link, Node, PartStats, SentinelList, NODE_CODE},
    sound,
    time::pick,
};

use estimate::{estimate, mount_rounds, Duration, Estimate};

const CAMPAIGN_MAP: usize = 0x1_47fc_6fd0;
const MAP_DIALOGUE: usize = 0x1_47fc_6fd2;
const MAP_DETACH: usize = 0x1_47fc_6fd6;
const MAP_TARGET_TOOL: usize = 0x1_47fc_6fd7;
const PLAYER_ESCADRA: usize = 0x1_43a1_3eb0;
const PROFILE_FLEETS_BEGIN: usize = 0x1f8;
const PROFILE_FLEETS_END: usize = 0x200;
const MAX_PROFILE_FLEETS: usize = 256;
/// Campaign hour clock (`hourCount_`); barrages and shell flight use map time, so
/// time compression advances them. `GAME_CLOCK` (0x143a119c0) is real game time.
const CAMPAIGN_HOUR_CLOCK: usize = 0x1_43a1_19c8;
const SECONDS_PER_HOUR: f64 = 3_600.0;
const GET_PROPERTY_LINKED_OBJECT: usize = 0x1_4021_b070;
const GET_STATS: usize = 0x1_4028_1e00;
const COPY_STRING: usize = 0x1_4000_56e0;
/// Map renderer's clean screen-space flush call (`VideoTask_RenderMapScene`, M11).
/// Take the firing position after the strategic fleet's movement for this frame.
const MAP_FRAME_SEAM: usize = 0x1_4030_dae4;
const FLUSH: usize = 0x1_4008_3b50;

const ESCADRA_CLASS: u16 = 0x147;
const LOCATION_CLASS: u16 = 0xc7;
const CREATURE_CLASS: u8 = 0x2f;
const CALIBER_180_MM: i32 = 25;
// Escadra and Location both inherit these Mark fields (not in highfleet-rs).
const MAP_POSITION: usize = 0x150;
const MAP_ALTITUDE: usize = 0x168;
const ESCADRA_READ_SPAN: usize = MAP_ALTITUDE + size_of::<f32>();
// GetPropertyLinkedObject inspects the Property's +0x100/+0x108 child-list header.
const LIVE_PROPERTY_SPAN: usize = offset_of!(Node, children) + size_of::<ShipProperties>();
// FireGun (0x140032f4a) reads the mount's RPM here, immediately beyond the modeled prefix.
const STAT_WEAPON_RATE: usize = 0x150;
const STAT_WEAPON_SPAN: usize =
    STAT_WEAPON_RATE + size_of::<f32>() - offset_of!(PartStats, caliber);

type ShipProperties = SentinelList<usize>;
type ShipProperty = SentinelListNode<usize>;

const _: () = {
    assert!(offset_of!(Node, children) + offset_of!(ShipProperties, head) == 0x100);
    assert!(offset_of!(Node, children) + offset_of!(ShipProperties, length) == 0x108);
    assert!(offset_of!(ShipProperty, value) == 0x10);
    assert!(size_of::<CVec<Link>>() == 3 * size_of::<usize>());
    assert!(offset_of!(PartStats, magazine_capacity) == 0x148);
    assert!(offset_of!(PartStats, reload_time) == 0x14c);
    assert!(size_of::<PartStats>() == STAT_WEAPON_RATE);
};

const BUTTON: &CStr = c"ALLY_SHELLING";
const FIRE: &CStr = c"ALLY_SHELL_FIRE";
const CANCEL: &CStr = c"ALLY_SHELL_EXIT";
const MAP_MARKER: &str = "MAP_LOCATION";
const PREVIEW_REFRESH_MS: u64 = 200;
const CLICK_DEBOUNCE_MS: u64 = 120;
const NOTICE_MS: u64 = 2_500;
const SOUND_GAP_MS: usize = 6_800;
const SOUND_JITTER_MS: usize = 1_000;
const OPENING_SHOT_GAP_MS: u64 = 85;
// Center-origin 1920×1080 cockpit coordinates. The label stays right-aligned
// above the map's coordinate readout; FIRE and CANCEL sit to its left.
const BUTTON_RIGHT: f32 = 407.0;
const BUTTON_Y: f32 = 307.0;
const BUTTON_HEIGHT: f32 = 42.0;
const CLOSED_WIDTH: f32 = 158.0;
const STATUS_WIDTH: f32 = 232.0;
const PREVIEW_WIDTH: f32 = 560.0;
const FIRE_WIDTH: f32 = 96.0;
const CANCEL_WIDTH: f32 = 108.0;
const BUTTON_GAP: f32 = 10.0;
// VideoTask_RenderMapScene walks stages 60..70. Stage 67 is over the cockpit
// panels (65/66); the default 60 shares an early layer with the city markers.
const COCKPIT_CONTROL_STAGE: i32 = 67;

static ENABLED: AtomicBool = AtomicBool::new(false);
static BUTTON_GUI: AtomicUsize = AtomicUsize::new(0);
static FIRE_GUI: AtomicUsize = AtomicUsize::new(0);
static CANCEL_GUI: AtomicUsize = AtomicUsize::new(0);
static MENU: Mutex<Menu> = Mutex::new(Menu {
    phase: Phase::Closed,
    barrage: None,
    notice: None,
    last_clock: None,
    last_refresh: None,
    last_control_click: None,
});

struct Fleet {
    identity: (usize, i64),
    position: [f32; 2],
    altitude: f32,
    battery: Battery,
}

#[derive(Default)]
struct Battery {
    rounds: u32,
    mounts: u32,
    barrels: u32,
    sound_weight: u32,
}

struct Barrage {
    fleet: (usize, i64),
    origin: [f32; 2],
    start_clock: Option<f64>,
    firing_seconds: f64,
    flight_seconds: f64,
    battery: Battery,
    hit_chance: f64,
    effective_hits: u32,
    next_report: Instant,
    opening_reports_left: u32,
    burst_reports_left: usize,
    seed: u64,
    city: [f32; 2],
}

enum BarrageReport {
    Opening(u32),
    Burst(u32),
}

enum Phase {
    Closed,
    PickCity {
        fleet: (usize, i64),
    },
    Preview {
        fleet: (usize, i64),
        city: [f32; 2],
        duration: Duration,
    },
}

struct Menu {
    phase: Phase,
    barrage: Option<Barrage>,
    notice: Option<(Instant, String)>,
    last_clock: Option<f64>,
    last_refresh: Option<Instant>,
    last_control_click: Option<Instant>,
}

impl Menu {
    fn duration(&self) -> Duration {
        match self.phase {
            Phase::Preview { duration, .. } => duration,
            _ => Duration::default(),
        }
    }

    fn check_fleet(&mut self, current: Option<&Fleet>) {
        let planned = match self.phase {
            Phase::Closed => return,
            Phase::PickCity { fleet } | Phase::Preview { fleet, .. } => fleet,
        };
        if current.is_none_or(|fleet| fleet.identity != planned) {
            self.phase = Phase::Closed;
        }
    }

    fn notice(&mut self, now: Instant, text: impl Into<String>) {
        self.notice = Some((now, text.into()));
        self.last_refresh = None;
    }

    fn notice_text(&self, now: Instant) -> Option<String> {
        self.notice
            .as_ref()
            .filter(|(at, _)| now.duration_since(*at).as_millis() < NOTICE_MS.into())
            .map(|(_, text)| text.clone())
    }
}

pub(crate) fn configure(enabled: bool) {
    ENABLED.store(enabled, Relaxed);
}

/// Installs the post-simulation map visualization and tactical raid.
///
/// # Safety
/// Call during serialized initialization after verifying the Steam 1.163 image base.
/// The fixed seam must remain mapped; patchy rejects a conflicting patch.
pub(crate) unsafe fn install(session: &mut PatchSession) -> Result<(), String> {
    let mut map_frame = Trampoline::new();
    map_frame.absolute_call(FLUSH as *const ());
    map_frame.preserved_call(on_map_frame as *const (), &[], &[], ReturnType::None);
    map_frame.relative_jump(MAP_FRAME_SEAM + 5);
    session
        .detour_trampoline(MAP_FRAME_SEAM, 5, map_frame)
        .map_err(|error| error.to_string())?;
    encounter::install(session)
}

/// Runs after the map simulation and before drawing, on the game thread.
unsafe extern "C" fn on_map_frame() {
    if !ENABLED.load(Relaxed) || read::<u8>(CAMPAIGN_MAP) == 0 {
        return;
    }
    let (targeting_fleet, firing_fleet) = {
        let Ok(menu) = MENU.try_lock() else {
            return;
        };
        let targeting = match menu.phase {
            Phase::PickCity { fleet } => Some(fleet),
            _ => None,
        };
        (
            targeting,
            menu.barrage.as_ref().map(|barrage| barrage.fleet),
        )
    };
    // SAFETY: M11 runs on the game thread with the native line renderer active.
    // The menu snapshot holds no lock across renderer calls.
    let targeting_origin = targeting_fleet
        .filter(|fleet| map_ready() && read::<usize>(PLAYER_ESCADRA) == fleet.0)
        .and_then(|fleet| firing_fleet_position(fleet));
    if let Some(origin) = targeting_origin {
        visual::draw_targeting_arrow(origin);
    }
    // The barrage visualization continues even when a different fleet is selected.
    if let Some(fleet) = firing_fleet {
        visual::draw_with_clock(read(CAMPAIGN_HOUR_CLOCK), firing_fleet_position(fleet));
    }
}

/// The current profile's bounded fleet pointer slot range.
///
/// # Safety
/// Call from a live campaign callback. Use the range only in this callback, before any native
/// operation that could replace or mutate the fleet vector.
unsafe fn campaign_fleet_slots() -> Option<Range<usize>> {
    let get_profile: GetProfileFn = std::mem::transmute(GET_PROFILE);
    let profile = get_profile();
    if !plausible(profile) {
        return None;
    }
    let begin = rd::<usize>(profile + PROFILE_FLEETS_BEGIN)?;
    let end = rd::<usize>(profile + PROFILE_FLEETS_END)?;
    let bytes = end.checked_sub(begin)?;
    (plausible(begin)
        && bytes.is_multiple_of(size_of::<usize>())
        && bytes / size_of::<usize>() <= MAX_PROFILE_FLEETS
        && (bytes == 0 || readable_span(begin, bytes)))
    .then_some(begin..end)
}

/// Read the original battery's position, not the newly selected fleet's position.
///
/// # Safety
/// Called on the live map seam. Revalidate against the current selection or campaign roster
/// before reading the captured address; only scalar position is returned to the visual plan.
unsafe fn firing_fleet_position((fleet, id): (usize, i64)) -> Option<[f32; 2]> {
    if read::<usize>(PLAYER_ESCADRA) != fleet
        && !campaign_fleet_slots()?
            .step_by(size_of::<usize>())
            .any(|slot| read::<usize>(slot) == fleet)
    {
        return None;
    }
    if !plausible(fleet)
        || !readable_span(fleet, ESCADRA_READ_SPAN)
        || read::<u16>(fleet + NODE_CODE) != ESCADRA_CLASS
        || read::<i64>(fleet + offset_of!(Node, id)) != id
    {
        return None;
    }
    let position: [f32; 2] = read(fleet + MAP_POSITION);
    position
        .iter()
        .all(|value| value.is_finite())
        .then_some(position)
}

unsafe fn map_ready() -> bool {
    // SAFETY: fixed image globals for Steam 1.163. The game-thread caller serializes UI access.
    read::<u8>(CAMPAIGN_MAP) != 0
        && read::<u8>(MAP_DIALOGUE) == 0
        && read::<u8>(IN_COMBAT) == 0
        && read::<u8>(MAP_DETACH) == 0
        && read::<u8>(MAP_TARGET_TOOL) == 0
        && read::<u8>(SCENE_FLAGS[2]) == 0
        && read::<usize>(ACTIVE_DIALOG) == 0
}

fn remaining(barrage: &Barrage, clock: f64) -> Option<f64> {
    let start_clock = barrage.start_clock?;
    (clock.is_finite() && clock >= start_clock).then_some(
        barrage.firing_seconds + barrage.flight_seconds - (clock - start_clock) * SECONDS_PER_HOUR,
    )
}

fn countdown_text(seconds: f64) -> String {
    if seconds < 60.0 {
        format!("{seconds:.0} S")
    } else {
        format!("{:.1} MIN", seconds / 60.0)
    }
}

/// A live Creature view: a ship property in a fleet can also point at a map missile,
/// which must never reach part traversal or `CalculateStats`.
///
/// # Safety
/// Call on the game thread with a pointer obtained from a live fleet property. This is a
/// class/span check, not proof that the object survives beyond the current callback.
pub(super) unsafe fn is_ship(ship: usize) -> bool {
    plausible(ship)
        && readable_span(ship, size_of::<Creature>())
        && read::<u8>(ship + NODE_CODE) & CREATURE_CLASS == CREATURE_CLASS
}

/// One borrowed target per fleet property, including zero for an unresolved
/// entry. Callers preparing a native encounter must not skip unresolved entries.
///
/// # Safety
/// Call on the game thread while `fleet` is a live Escadra. The returned pointers are
/// borrowed game objects and must be revalidated before each later native read.
unsafe fn fleet_ships(fleet: usize) -> Option<Vec<usize>> {
    if !plausible(fleet)
        || !readable_span(fleet, ESCADRA_READ_SPAN)
        || read::<u16>(fleet + NODE_CODE) != ESCADRA_CLASS
    {
        return None;
    }
    let children = fleet + offset_of!(Node, children);
    let count: usize = read(children + offset_of!(ShipProperties, length));
    if count == 0 || count > SHIP_LIST_LIMIT {
        return None;
    }
    let head: usize = read(children + offset_of!(ShipProperties, head));
    let mut entry: usize = rd(head)?;
    let linked: unsafe extern "C" fn(usize) -> usize =
        std::mem::transmute(GET_PROPERTY_LINKED_OBJECT);
    let mut ships = Vec::with_capacity(count);
    for _ in 0..count {
        if entry == head || !plausible(entry) || !readable_span(entry, size_of::<ShipProperty>()) {
            return None;
        }
        let property: usize = read(entry + offset_of!(ShipProperty, value));
        if !plausible(property) || !readable_span(property, LIVE_PROPERTY_SPAN) {
            return None;
        }
        // SAFETY: this is a Property from the fleet's live children list. The native
        // getter walks its own live links; it doesn't transfer ownership or mutate the fleet.
        ships.push(linked(property));
        entry = read(entry + offset_of!(ShipProperty, next));
    }
    (entry == head).then_some(ships)
}

/// Snapshot the active friendly fleet and the rounds its working 180 mm mounts can fire.
///
/// # Safety
/// Call only on the input thread during the live strategic map. The selected fleet
/// and its property links remain owned by the game and valid for this synchronous read.
unsafe fn selected_fleet(duration: Duration) -> Option<Fleet> {
    let fleet: usize = read(PLAYER_ESCADRA);
    if !plausible(fleet) || !readable_span(fleet, ESCADRA_READ_SPAN) {
        return None;
    }
    let position: [f32; 2] = read(fleet + MAP_POSITION);
    let altitude: f32 = read(fleet + MAP_ALTITUDE);
    if !position.iter().all(|coordinate| coordinate.is_finite()) || !altitude.is_finite() {
        return None;
    }
    let mut battery = Battery::default();
    for ship in fleet_ships(fleet)? {
        if !is_ship(ship) {
            continue;
        }
        let ships_guns = battery_stats(ship, duration)?;
        battery.rounds = battery.rounds.saturating_add(ships_guns.rounds);
        battery.mounts = battery.mounts.saturating_add(ships_guns.mounts);
        battery.barrels = battery.barrels.saturating_add(ships_guns.barrels);
        battery.sound_weight = battery.sound_weight.saturating_add(ships_guns.sound_weight);
    }
    (battery.rounds != 0).then(|| Fleet {
        identity: (fleet, read(fleet + offset_of!(Node, id))),
        position,
        altitude,
        battery,
    })
}

/// # Safety
/// `ship` is the current game-thread creature behind a fleet property. Parts and stat
/// records remain game-owned; this function only copies scalar fields and raw pointers.
unsafe fn battery_stats(ship: usize, duration: Duration) -> Option<Battery> {
    if !plausible(ship)
        || !readable_span(ship, size_of::<Creature>())
        || read::<u8>(ship + NODE_CODE) & CREATURE_CLASS != CREATURE_CLASS
    {
        return None;
    }
    let get_stats: unsafe extern "C" fn(i32) -> usize = std::mem::transmute(GET_STATS);
    let mut battery = Battery::default();
    for offset in [
        offset_of!(Creature, gun_group_0),
        offset_of!(Creature, gun_group_1),
    ] {
        let group = ship + offset;
        // CVec's pointers are private upstream: initialized begin, end, then capacity.
        let begin: usize = read(group);
        let end: usize = read(group + size_of::<usize>());
        let bytes = end.checked_sub(begin)?;
        if bytes == 0 {
            continue;
        }
        if !bytes.is_multiple_of(size_of::<Link>())
            || bytes / size_of::<Link>() > SHIP_LIST_LIMIT
            || !plausible(begin)
            || !readable_span(begin, bytes)
        {
            return None;
        }
        for link in (begin..end).step_by(size_of::<Link>()) {
            let gun: usize = read(link + offset_of!(Link, target));
            if !plausible(gun) || !readable_span(gun, offset_of!(Body, health) + size_of::<f32>()) {
                continue;
            }
            let health: f32 = read(gun + offset_of!(Body, health));
            if read::<i32>(gun + OFF_ACTIVE) == 0 || !health.is_finite() || health <= 0.0 {
                continue;
            }
            // SAFETY: this is a borrowed gun Body from one of the mutually exclusive
            // native gun groups. GetStats bounds-checks its scalar part index.
            let stats = get_stats(read(gun + offset_of!(Body, part_index)));
            if !plausible(stats)
                || !readable_span(stats + offset_of!(PartStats, caliber), STAT_WEAPON_SPAN)
                || read::<i32>(stats + offset_of!(PartStats, caliber)) != CALIBER_180_MM
            {
                continue;
            }
            let capacity = read::<i32>(stats + offset_of!(PartStats, magazine_capacity));
            if let Some(mount) = mount_rounds(
                duration,
                capacity,
                read(stats + offset_of!(PartStats, reload_time)),
                read(stats + STAT_WEAPON_RATE),
            ) {
                // Steam Libraries/OL.seria: these are the stock Sarmat and Squall OIDs.
                // Magazine capacity remains independent of barrel count for modified guns.
                let barrels = match crate::memory::native_string(gun + offset_of!(Body, oid))
                    .as_deref()
                    .map(CStr::to_bytes)
                {
                    Some(b"MDL_CANNON_180_2") => 2,
                    Some(b"MDL_CANNON_305_2") => 6,
                    _ => 1,
                };
                battery.rounds = battery.rounds.saturating_add(mount);
                battery.mounts += 1;
                battery.barrels += barrels;
                battery.sound_weight = battery.sound_weight.saturating_add(capacity.max(0) as u32);
            }
        }
    }
    Some(battery)
}

/// Called by the existing game-thread GUI input callback, before rendering.
pub(crate) unsafe fn tick() {
    if !ENABLED.load(Relaxed) {
        return;
    }
    // Leaving the campaign map discards the untracked preview and barrage state.
    if read::<u8>(CAMPAIGN_MAP) == 0 {
        visual::clear();
        if let Ok(mut menu) = MENU.try_lock() {
            menu.phase = Phase::Closed;
            menu.barrage = None;
        }
        return;
    }
    let active = map_ready();
    let Ok(mut menu) = MENU.try_lock() else {
        return;
    };
    let wall_now = Instant::now();
    let clock: f64 = read(CAMPAIGN_HOUR_CLOCK);
    let clock_reset = menu
        .last_clock
        .is_some_and(|previous| clock.is_finite() && previous.is_finite() && clock < previous);
    menu.last_clock = clock.is_finite().then_some(clock);
    if clock_reset {
        // A loaded campaign clock must not inherit the previous firing plan.
        menu.barrage = None;
        menu.phase = Phase::Closed;
        menu.last_refresh = None;
        visual::clear();
    }
    if let Some(barrage) = menu.barrage.as_mut() {
        if barrage.start_clock.is_none()
            && active
            && clock.is_finite()
            && wall_now >= barrage.next_report
        {
            let battery = &barrage.battery;
            let effective =
                estimate::effective_rounds(barrage.seed, battery.rounds, barrage.hit_chance);
            barrage.effective_hits = effective.len() as u32;
            barrage.start_clock = Some(clock);
            log::info!(
                "CCU: shelling barrage started: {:.0} min, {} rounds, {} mounts, {} barrels, {:.2} expected hits at {:?}",
                barrage.firing_seconds / 60.0,
                battery.rounds, battery.mounts, battery.barrels,
                f64::from(battery.rounds) * barrage.hit_chance, barrage.city,
            );
            visual::begin(
                barrage.origin,
                barrage.city,
                clock,
                barrage.firing_seconds,
                visual::Composition::new(
                    battery.mounts,
                    battery.rounds,
                    battery.sound_weight,
                    barrage.firing_seconds,
                ),
                effective,
            );
        }
        match remaining(barrage, clock) {
            Some(left) if active && left <= 0.0 => {
                let city = barrage.city;
                let hits = barrage.effective_hits;
                let volume = encounter::Bombardment {
                    hits,
                    rounds: barrage.battery.rounds,
                    seed: barrage.seed,
                    incoming_direction: if barrage.origin[0] <= city[0] {
                        -1.0
                    } else {
                        1.0
                    },
                };
                menu.barrage = None;
                menu.phase = Phase::Closed;
                drop(menu);
                let message = if hits == 0 {
                    "BARRAGE: NO EFFECTIVE HITS"
                } else {
                    let fleets = strike::garrison_fleets(city);
                    if fleets.is_empty() {
                        "BARRAGE: NO GARRISON"
                    } else if encounter::start(&fleets, volume) {
                        return; // The level loader may have invalidated the campaign GUI.
                    } else {
                        "BOMBARDMENT COULD NOT START"
                    }
                };
                log::info!("CCU: {message} at {city:?} ({hits} effective hits)");
                if let Ok(mut menu) = MENU.try_lock() {
                    menu.notice(wall_now, message);
                }
                return;
            }
            None if barrage.start_clock.is_some() => {
                menu.barrage = None;
                menu.phase = Phase::Closed;
                visual::clear();
                menu.notice(wall_now, "BARRAGE LOST");
            }
            _ => {}
        }
    }
    let report = menu.barrage.as_mut().and_then(|barrage| {
        if !active || barrage.start_clock.is_none() || wall_now < barrage.next_report {
            return None;
        }
        let weight = barrage.battery.sound_weight;
        if barrage.opening_reports_left > 0 {
            barrage.opening_reports_left -= 1;
            let gap = if barrage.opening_reports_left > 0 {
                OPENING_SHOT_GAP_MS
            } else {
                (SOUND_GAP_MS + pick(SOUND_JITTER_MS)) as u64
            };
            barrage.next_report = wall_now + WallDuration::from_millis(gap);
            return Some(BarrageReport::Opening(weight));
        }
        if read::<usize>(PLAYER_ESCADRA) != barrage.fleet.0
            || !remaining(barrage, clock).is_some_and(|left| left > barrage.flight_seconds)
        {
            return None;
        }
        if barrage.burst_reports_left == 0 {
            barrage.burst_reports_left = 2 + pick(4);
        }
        barrage.burst_reports_left -= 1;
        let gap = if barrage.burst_reports_left > 0 {
            OPENING_SHOT_GAP_MS
        } else {
            (SOUND_GAP_MS + pick(SOUND_JITTER_MS)) as u64
        };
        barrage.next_report = wall_now + WallDuration::from_millis(gap);
        Some(BarrageReport::Burst(weight))
    });
    if let Some(report) = report {
        drop(menu);
        // SAFETY: live map GUI; no MENU lock is held across the native sound call.
        match report {
            BarrageReport::Opening(weight) => sound::shelling_opening_shot(weight),
            BarrageReport::Burst(weight) => sound::shelling_burst_shot(weight),
        }
        let Ok(locked) = MENU.try_lock() else {
            return;
        };
        menu = locked;
    }
    if active {
        if menu.last_refresh.is_some_and(|last| {
            wall_now.duration_since(last).as_millis() < PREVIEW_REFRESH_MS.into()
        }) {
            return;
        }
        menu.last_refresh = Some(wall_now);
    } else {
        menu.last_refresh = None;
    }
    let fleet = if active {
        selected_fleet(menu.duration())
    } else {
        None
    };
    menu.check_fleet(fleet.as_ref());
    let selected_barrage = menu.barrage.as_ref().filter(|barrage| {
        fleet
            .as_ref()
            .is_some_and(|fleet| fleet.identity == barrage.fleet)
    });
    let has_barrage = selected_barrage.is_some();
    let preparing = selected_barrage.is_some_and(|barrage| barrage.start_clock.is_none());
    let firing_left = selected_barrage.and_then(|barrage| remaining(barrage, clock));
    let launch_left = selected_barrage
        .zip(firing_left)
        .map(|(barrage, left)| left - barrage.flight_seconds);
    let notice = menu.notice_text(wall_now);
    let phase = match menu.phase {
        Phase::Closed => Display::Closed,
        Phase::PickCity { .. } => Display::PickCity,
        Phase::Preview { city, duration, .. } => Display::Preview { city, duration },
    };
    drop(menu);

    let previewing = fleet.is_some() && matches!(phase, Display::Preview { .. }) && !has_barrage;
    let has_notice = notice.is_some();
    let text = fleet.as_ref().map(|fleet| {
        if preparing {
            "PREPARING TO FIRE".into()
        } else if let Some(left) = firing_left {
            if let Some(launch) = launch_left.filter(|&launch| launch > 0.0) {
                format!("FIRING  {} LEFT", countdown_text(launch))
            } else if left > 0.0 {
                format!("LANDING  {} LEFT", countdown_text(left))
            } else {
                "SHELLS LANDING".into()
            }
        } else if let Some(notice) = notice {
            notice
        } else {
            match phase {
                Display::Closed => "SHELLING".into(),
                Display::PickCity => "SELECT A CITY ON THE MAP".into(),
                Display::Preview { city, duration } => {
                    let distance = (fleet.position[0] - city[0]).hypot(fleet.position[1] - city[1]);
                    match estimate(distance, fleet.altitude, fleet.battery.rounds) {
                        Some(Estimate::OutOfRange {
                            distance_km,
                            range_km,
                        }) => format!("OUT OF RANGE: {distance_km:.0} / {range_km:.0} KM"),
                        Some(Estimate::InRange {
                            rounds,
                            first_impact_minutes,
                            expected_hits,
                            any_hit_percent,
                        }) => {
                            // The expected count is the scaling quantity; a near-certain
                            // "any hit" percentage would round to 100 for every duration.
                            let chance = if any_hit_percent < 99.5 {
                                format!(" | {any_hit_percent:.0}%")
                            } else {
                                String::new()
                            };
                            format!(
                                "{} MIN: {} HE | ETA {:.1}M | {:.1} HITS{}",
                                duration.minutes(),
                                rounds,
                                first_impact_minutes,
                                expected_hits,
                                chance
                            )
                        }
                        None => "INVALID FIRING SOLUTION".into(),
                    }
                }
            }
        }
    });

    if text.is_none()
        && BUTTON_GUI.load(Relaxed) == 0
        && FIRE_GUI.load(Relaxed) == 0
        && CANCEL_GUI.load(Relaxed) == 0
    {
        return;
    }
    // SAFETY: the shared scanner and helpers validate the GUI registry on this game-thread
    // callback. Each button addition is followed by a fresh scan before the next one.
    let scan = gui::scan_gui_list();
    if let Some(text) = text {
        let width = if has_barrage {
            STATUS_WIDTH
        } else if has_notice {
            PREVIEW_WIDTH
        } else {
            match phase {
                Display::Closed => CLOSED_WIDTH,
                Display::PickCity => STATUS_WIDTH,
                Display::Preview { .. } => PREVIEW_WIDTH,
            }
        };
        paint_button(BUTTON, &BUTTON_GUI, &scan, &text, BUTTON_RIGHT, width);
    } else {
        hide_button(BUTTON, &BUTTON_GUI, &scan);
    }
    if previewing {
        let scan = gui::scan_gui_list();
        paint_button(
            FIRE,
            &FIRE_GUI,
            &scan,
            "FIRE",
            BUTTON_RIGHT - PREVIEW_WIDTH - BUTTON_GAP,
            FIRE_WIDTH,
        );
    } else {
        hide_button(FIRE, &FIRE_GUI, &scan);
    }
    let planning = fleet.is_some() && (!matches!(phase, Display::Closed) || has_barrage);
    if planning || CANCEL_GUI.load(Relaxed) != 0 {
        let scan = gui::scan_gui_list();
        if planning {
            paint_button(
                CANCEL,
                &CANCEL_GUI,
                &scan,
                if has_barrage { "ABORT" } else { "CANCEL" },
                BUTTON_RIGHT - PREVIEW_WIDTH - 2.0 * BUTTON_GAP - FIRE_WIDTH,
                CANCEL_WIDTH,
            );
        } else {
            hide_button(CANCEL, &CANCEL_GUI, &scan);
        }
    }
}

#[derive(Clone, Copy)]
enum Display {
    Closed,
    PickCity,
    Preview { city: [f32; 2], duration: Duration },
}

/// # Safety
/// The scan belongs to this game-thread callback and no native GUI rebuild may intervene.
unsafe fn paint_button(
    name: &CStr,
    kept: &AtomicUsize,
    scan: &gui::GuiScan,
    text: &str,
    right: f32,
    width: f32,
) {
    let x = right - width / 2.0;
    let position = [x, BUTTON_Y];
    let dimensions = [width, BUTTON_HEIGHT];
    let Some((gui, created)) = gui::get_or_make_with(
        name.to_bytes_with_nul(),
        kept,
        &scan.elements,
        scan.complete,
        || {
            let pointer = gui::allocate_gui();
            if pointer.is_null() {
                return pointer;
            }
            // SAFETY: native constructor produced a fresh GUI. Init copies the static strings;
            // attach transfers the control to the game's root before it can receive input.
            let vtable: usize = read(pointer as usize);
            let init: gui::GuiInitFn = std::mem::transmute(read::<usize>(vtable + 0x2c0));
            init(
                pointer,
                name.as_ptr().cast(),
                x,
                BUTTON_Y,
                width,
                BUTTON_HEIGHT,
                c"".as_ptr().cast(),
                c"gui_std".as_ptr().cast(),
                c"gui_std_focus".as_ptr().cast(),
                c"gui_std_click".as_ptr().cast(),
                c"fixed_16".as_ptr().cast(),
                1.0,
            );
            pointer.add(offset_of!(Body, gui)).write(1);
            pointer.add(offset_of!(Body, scalable)).write(0);
            pointer.add(offset_of!(Body, focusable)).write(1);
            pointer.add(offset_of!(Gui, clickable)).write(1);
            pointer
                .add(offset_of!(Body, stage))
                .cast::<i32>()
                .write_unaligned(COCKPIT_CONTROL_STAGE);
            gui::attach_gui(pointer);
            gui::load_gui(pointer as usize);
            pointer
        },
    ) else {
        return;
    };
    if created {
        log::debug!("shelling GUI ready: {name:?} ({x}, {BUTTON_Y}) stage {COCKPIT_CONTROL_STAGE}");
    } else {
        gui::move_gui_rect(gui, position, dimensions);
    }
    // SAFETY: the live root-owned GUI contains a native text std::string. Assignment
    // synchronously copies exactly these ASCII bytes; Rust never owns its storage.
    let assign: unsafe extern "C" fn(usize, *const u8, usize) -> usize =
        std::mem::transmute(COPY_STRING);
    assign(gui + offset_of!(Gui, text), text.as_ptr(), text.len());
}

unsafe fn hide_button(name: &CStr, kept: &AtomicUsize, scan: &gui::GuiScan) {
    let gui = kept.load(Relaxed);
    if gui::listed_shown(&scan.elements, gui, name.to_bytes_with_nul()) {
        gui::set_gui_active(gui, false);
    } else if scan.complete && !scan.elements.contains(&gui) {
        kept.store(0, Relaxed);
    }
}

/// Handles owned controls and left-clicked city markers.
///
/// # Safety
/// `gui` is the borrowed live element handed to the game's ClickFunc on the input thread.
pub(crate) unsafe fn on_click(name: &str, gui: usize, button: usize) -> bool {
    if !ENABLED.load(Relaxed) {
        return false;
    }
    let main = name.as_bytes() == BUTTON.to_bytes();
    let fire = name.as_bytes() == FIRE.to_bytes();
    let cancel = name.as_bytes() == CANCEL.to_bytes();
    let owned = main || fire || cancel;
    let Ok(mut menu) = MENU.try_lock() else {
        return owned;
    };
    if read::<u8>(CAMPAIGN_MAP) == 0 {
        menu.phase = Phase::Closed;
        menu.barrage = None;
    }
    if !map_ready() || (!owned && (name.starts_with("KEY_ESC") || name.starts_with("QUIT_"))) {
        menu.phase = Phase::Closed;
        return owned;
    }
    if button as u32 != 1 {
        return owned;
    }
    if !owned && (name != MAP_MARKER || matches!(menu.phase, Phase::Closed)) {
        return false;
    }
    let now = Instant::now();
    if owned {
        if menu
            .last_control_click
            .is_some_and(|last| now.duration_since(last).as_millis() < CLICK_DEBOUNCE_MS.into())
        {
            return true;
        }
        menu.last_control_click = Some(now);
    }
    menu.last_refresh = None;
    let fleet = selected_fleet(menu.duration());
    menu.check_fleet(fleet.as_ref());
    let Some(fleet) = fleet else {
        return owned;
    };
    let firing_here = menu
        .barrage
        .as_ref()
        .is_some_and(|barrage| barrage.fleet == fleet.identity);
    if cancel {
        if firing_here {
            menu.barrage = None;
            visual::clear();
            menu.notice(now, "BARRAGE ABORTED");
        }
        menu.phase = Phase::Closed;
        drop(menu);
        if firing_here {
            // SAFETY: game-thread click callback outside MENU lock; GUI root is live.
            sound::shelling_radio_abort();
        }
        return true;
    }
    if fire {
        let Phase::Preview {
            fleet: planned,
            city,
            duration,
        } = menu.phase
        else {
            return true;
        };
        if firing_here {
            return true;
        }
        if menu.barrage.is_some() {
            menu.notice(now, "ANOTHER BARRAGE IS UNDER WAY");
            return true;
        }
        if fleet.identity != planned {
            menu.phase = Phase::Closed;
            return true;
        }
        let distance = (fleet.position[0] - city[0]).hypot(fleet.position[1] - city[1]);
        let Some(Estimate::InRange { .. }) =
            estimate(distance, fleet.altitude, fleet.battery.rounds)
        else {
            menu.notice(now, "OUT OF RANGE");
            return true;
        };
        let clock: f64 = read(CAMPAIGN_HOUR_CLOCK);
        if !clock.is_finite() {
            menu.notice(now, "NO CAMPAIGN CLOCK");
            return true;
        }
        if fleet.battery.rounds > estimate::MAX_SIMULATED_ROUNDS {
            menu.notice(now, "FIRING PLAN UNAVAILABLE");
            return true;
        }
        let firing_seconds = f64::from(duration.minutes()) * 60.0;
        let hit_chance = estimate::round_hit_chance(distance, fleet.altitude);
        let seed = clock.to_bits()
            ^ (fleet.identity.1 as u64).rotate_left(17)
            ^ u64::from(city[0].to_bits()).rotate_left(32)
            ^ u64::from(city[1].to_bits()).rotate_left(47);
        let opening_reports_left = fleet.battery.barrels;
        menu.barrage = Some(Barrage {
            fleet: fleet.identity,
            origin: fleet.position,
            start_clock: None,
            firing_seconds,
            flight_seconds: estimate::flight_seconds(distance),
            battery: fleet.battery,
            hit_chance,
            effective_hits: 0,
            next_report: Instant::now() + WallDuration::from_millis(sound::shelling_prelude_ms()),
            opening_reports_left,
            burst_reports_left: 0,
            seed,
            city,
        });
        drop(menu);
        // SAFETY: game-thread click callback outside MENU lock; GUI root is live.
        sound::shelling_windup();
        sound::shelling_radio_fire();
        return true;
    }
    if firing_here {
        return true;
    }
    if main {
        if matches!(menu.phase, Phase::Closed) {
            menu.phase = Phase::PickCity {
                fleet: fleet.identity,
            };
        } else if let Phase::Preview { duration, .. } = &mut menu.phase {
            *duration = duration.next();
        }
        return true;
    }
    if matches!(menu.phase, Phase::Closed) {
        return false;
    }
    // SAFETY: the game's live MAP_LOCATION button carries its current Location in gui_l1.
    // Check class and span before taking a by-value coordinate snapshot; never retain the pointer.
    let location: usize = read(gui + offset_of!(Gui, gui_l1) + offset_of!(Link, target));
    if plausible(location)
        && readable_span(location, MAP_POSITION + size_of::<[f32; 2]>())
        && read::<u16>(location + NODE_CODE) == LOCATION_CLASS
    {
        let city: [f32; 2] = read(location + MAP_POSITION);
        if city.iter().all(|coordinate| coordinate.is_finite()) {
            let duration = menu.duration();
            menu.phase = Phase::Preview {
                fleet: fleet.identity,
                city,
                duration,
            };
        }
    }
    true
}
