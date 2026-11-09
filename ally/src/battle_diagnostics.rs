//! Bounded, callback-local battle probes and the opt-in `ALLY track` diagnostic.
//!
//! Every native pointer here is validated against the current creature list before it is followed; the cached
//! probe identities are never treated as proof of liveness. The list view itself lives only for one callback.

use std::{
    mem::{offset_of, size_of},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Mutex,
    },
};

use crate::native_layout::{Body, Creature};
use crate::{
    diag, plausible, read, CLOCK, CREATURES_BEGIN, CREATURES_END, CREATURE_LIST_LIMIT, LINK_STRIDE,
    OFF_ACTIVE, OFF_AGGRESSION, OFF_AI_FLAG, OFF_CODE, OFF_FLAGS, OFF_MODE_D70, OFF_POS_X,
    OFF_POS_Y, OFF_SIDE, OFF_STATE, OFF_TARGET, SHIP_LIST_LIMIT,
};

// --- probe fields (all [code]/guess, see the findings note) ---
const OFF_RETARGET: usize = offset_of!(Creature, retarget_timer); // f32 seconds until the next target re-roll
const OFF_LAUNCH_CD: usize = 0xd60; // f32 ship launch cooldown
const OFF_LAUNCH_COUNT: usize = 0xc98; // i32 per-ship launch counter
const OFF_ZENITH: (usize, usize) = (
    offset_of!(Creature, anti_ship_launchers),
    offset_of!(Creature, anti_ship_launchers) + size_of::<usize>(),
); // anti-ship launcher list
const OFF_GUNS: (usize, usize) = (
    offset_of!(Creature, ammo_consumers),
    offset_of!(Creature, ammo_consumers) + size_of::<usize>(),
); // ammo-consumer list, the guns and launchers that draw rounds
const OFF_PART_COOLDOWN: usize = 0x848; // f32; highfleet-rs models this word as Body::mdl_covered
const OFF_PART_RESERVE: usize = offset_of!(Body, weapon_ammo_amount); // i32
const OFF_PART_LOADED: usize = offset_of!(Body, weapon_ammo_loaded); // f32
const OFF_WAYPOINT_X: usize = offset_of!(Creature, waypoint_x); // f32 steering waypoint written by the AI tick
const OFF_WAYPOINT_Y: usize = offset_of!(Creature, waypoint_y);

/// The ally being followed in the log (0 = none), the game time of its next track line and of the last one.
static TRACK_ALLY: AtomicUsize = AtomicUsize::new(0);
static TRACK_NEXT: AtomicU64 = AtomicU64::new(0);
static TRACK_END: AtomicU64 = AtomicU64::new(0);

/// Per-ship activity counters for the ally/enemy comparison.
#[derive(Default)]
struct Probe {
    who: usize,
    t0: f64,
    last_pos: (f32, f32),
    last_t: f64,
    last_zenith: f32,
    last_guns: f32,
    launches: u32,
    shots: u32,
}

static PROBE_ALLY: Mutex<Probe> = Mutex::new(Probe {
    who: 0,
    t0: 0.0,
    last_pos: (0.0, 0.0),
    last_t: 0.0,
    last_zenith: 0.0,
    last_guns: 0.0,
    launches: 0,
    shots: 0,
});
static PROBE_ENEMY: Mutex<Probe> = Mutex::new(Probe {
    who: 0,
    t0: 0.0,
    last_pos: (0.0, 0.0),
    last_t: 0.0,
    last_zenith: 0.0,
    last_guns: 0.0,
    launches: 0,
    shots: 0,
});

/// Clears the tracking target and both probe identities for a new battle.
///
/// A restart can reuse a freed ship's address for a GUI object whose active field also happens to
/// be nonzero, so cached probe pointers never survive the battle boundary.
pub(crate) fn reset() {
    TRACK_ALLY.store(0, Ordering::Relaxed);
    if let (Ok(mut ally_probe), Ok(mut enemy_probe)) = (PROBE_ALLY.lock(), PROBE_ENEMY.lock()) {
        *ally_probe = Probe::default();
        *enemy_probe = Probe::default();
    }
}

/// Starts the 60-second `ALLY track` window for the first ally deployed in this battle.
///
/// # Safety
/// Reads the game clock; call from the game thread at the point the ally is deployed.
pub(crate) unsafe fn start_tracking(ally: usize) {
    TRACK_ALLY.store(ally, Ordering::Relaxed);
    TRACK_NEXT.store(0, Ordering::Relaxed);
    TRACK_END.store((read::<f64>(CLOCK) + 60.0).to_bits(), Ordering::Relaxed);
}

/// (entries, sum of loaded rounds, sum of reserve rounds, entries off cooldown) of a part list.
unsafe fn part_list(ship: usize, list: (usize, usize)) -> (usize, f32, i64, usize) {
    let (b, e): (usize, usize) = (read(ship + list.0), read(ship + list.1));
    if b < 0x10000 || e < b || (e - b) / LINK_STRIDE > SHIP_LIST_LIMIT {
        return (0, 0.0, 0, 0);
    }
    let (mut n, mut loaded, mut reserve, mut ready) = (0, 0.0f32, 0i64, 0);
    for a in (b..e).step_by(LINK_STRIDE) {
        let part: usize = read(a + 8);
        if part == 0 {
            continue;
        }
        n += 1;
        loaded += read::<f32>(part + OFF_PART_LOADED);
        reserve += read::<i32>(part + OFF_PART_RESERVE) as i64;
        if read::<f32>(part + OFF_PART_COOLDOWN) <= 0.0 {
            ready += 1;
        }
    }
    (n, loaded, reserve, ready)
}

/// Zenith launcher and gun entry counts, for the `ALLY loadout` line.
///
/// # Safety
/// `ship` must be the ally that was just deployed, on the game thread.
pub(crate) unsafe fn loadout_counts(ship: usize) -> (usize, usize) {
    (part_list(ship, OFF_ZENITH).0, part_list(ship, OFF_GUNS).0)
}

impl Probe {
    fn reset(&mut self, who: usize, now: f64) {
        *self = Probe {
            who,
            t0: now,
            last_t: now,
            ..Default::default()
        };
        if who != 0 {
            unsafe {
                self.last_pos = (read(who + OFF_POS_X), read(who + OFF_POS_Y));
                self.last_zenith = part_list(who, OFF_ZENITH).1;
                self.last_guns = part_list(who, OFF_GUNS).1;
            }
        }
    }

    /// Called every frame: counts a launch whenever the launchers' loaded total drops, a shot
    /// whenever the weapons' loaded total drops (a reload raises it again, which is ignored).
    unsafe fn count(&mut self) {
        if self.who == 0 {
            return;
        }
        let zenith = part_list(self.who, OFF_ZENITH).1;
        let guns = part_list(self.who, OFF_GUNS).1;
        if zenith < self.last_zenith - 0.5 {
            self.launches += 1;
        }
        if guns < self.last_guns - 0.5 {
            self.shots += (self.last_guns - guns).round() as u32;
        }
        self.last_zenith = zenith;
        self.last_guns = guns;
    }

    /// One text line for the log.
    unsafe fn line(&mut self, tag: &str, now: f64, creatures: &ProbeCreatures) -> String {
        let ship = self.who;
        let pos = (read::<f32>(ship + OFF_POS_X), read::<f32>(ship + OFF_POS_Y));
        let dt = (now - self.last_t).max(0.001) as f32;
        let speed =
            ((pos.0 - self.last_pos.0).powi(2) + (pos.1 - self.last_pos.1).powi(2)).sqrt() / dt;
        self.last_pos = pos;
        self.last_t = now;
        let target: usize = read(ship + OFF_TARGET);
        // A target link alone does not establish a live creature. Non-creature or
        // stale targets have an unknown distance; never follow them for logging.
        let target_dist = if probe_ship_in_battle(target, creatures) {
            ((pos.0 - read::<f32>(target + OFF_POS_X)).powi(2)
                + (pos.1 - read::<f32>(target + OFF_POS_Y)).powi(2))
            .sqrt()
        } else {
            -1.0
        };
        let (zn, zl, zr, zready) = part_list(ship, OFF_ZENITH);
        let (gn, gl, gr, gready) = part_list(ship, OFF_GUNS);
        let flags: [u8; 5] = read(ship + OFF_FLAGS);
        format!(
            "ALLY probe {tag} t={:.0} c={ship:#x} pos=({:.0},{:.0}) speed={speed:.0} state={} mode={} target={target:#x} target_dist={target_dist:.0} aggr={:.2} retarget={:.1} \
             flags={flags:?} d60={:.1} launch_cnt={} | zenith n={zn} loaded={zl:.0} reserve={zr} ready={zready} launches={} | guns n={gn} loaded={gl:.0} reserve={gr} ready={gready} shots={}",
            now - self.t0,
            pos.0,
            pos.1,
            read::<i32>(ship + OFF_STATE),
            read::<i32>(ship + OFF_MODE_D70),
            read::<f32>(ship + OFF_AGGRESSION),
            read::<f32>(ship + OFF_RETARGET),
            read::<f32>(ship + OFF_LAUNCH_CD),
            read::<i32>(ship + OFF_LAUNCH_COUNT),
            self.launches,
            self.shots,
        )
    }
}

/// Current creature identities for one diagnostic callback. Never retained across ticks.
/// An unreadable or malformed list is not evidence that any cached object is alive.
struct ProbeCreatures {
    begin: usize,
    end: usize,
}

impl ProbeCreatures {
    /// # Safety
    /// The validated native span must remain alive and unchanged on the game thread. Only
    /// scalar pointer values are borrowed; callers must establish membership before following one.
    unsafe fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        (self.begin..self.end).step_by(LINK_STRIDE).map(|entry| {
            // SAFETY: construction checked every page of the bounded list, and the caller
            // guarantees its lifetime for this callback. This does not copy a native Link.
            unsafe { read::<usize>(entry + 8) }
        })
    }
}

/// Validate a bounded, callback-local view of the current native creature list.
///
/// # Safety
/// Call only from the battle hook on the game thread, while the native creature
/// list and its objects cannot be removed concurrently. No native ownership is taken.
unsafe fn probe_creatures() -> Option<ProbeCreatures> {
    let (Some(begin), Some(end)) = (
        diag::rd::<usize>(CREATURES_BEGIN),
        diag::rd::<usize>(CREATURES_END),
    ) else {
        return None;
    };
    if begin < 0x10000 || end < begin {
        return None;
    }
    let list_bytes = end - begin;
    if !list_bytes.is_multiple_of(LINK_STRIDE) || list_bytes / LINK_STRIDE > CREATURE_LIST_LIMIT {
        return None;
    }
    // SAFETY: the header has been range/stride/cap checked above. This is the game-thread
    // battle callback, with no intervening native mutations during diagnostic collection.
    if !diag::readable_span(begin, list_bytes) {
        return None;
    }
    Some(ProbeCreatures { begin, end })
}

/// Validate membership before inspecting any cached pointer; readable memory is not identity.
///
/// # Safety
/// `creatures` must be this callback's validated native list view, on the game thread,
/// with no intervening native operation that can destroy or replace its objects.
unsafe fn probe_ship_in_battle(ship: usize, creatures: &ProbeCreatures) -> bool {
    if !plausible(ship) || !creatures.iter().any(|current| current == ship) {
        return false;
    }
    diag::rd::<u8>(ship + OFF_CODE).is_some_and(|class| class & 0x2f == 0x2f)
        && diag::rd::<i32>(ship + OFF_ACTIVE).is_some_and(|active| active != 0)
        && diag::rd::<i32>(ship + OFF_STATE).is_some_and(|state| !matches!(state, 6 | 7))
}

/// The enemy nearest to `from`: side below zero, in the level.
///
/// # Safety
/// `from` must be a current creature validated on the game thread by the caller.
unsafe fn nearest_enemy(from: usize, creatures: &ProbeCreatures) -> usize {
    let (fx, fy) = (read::<f32>(from + OFF_POS_X), read::<f32>(from + OFF_POS_Y));
    let mut best = (usize::MAX as f32, 0usize);
    for p in creatures.iter() {
        if !plausible(p)
            || !diag::rd::<u8>(p + OFF_CODE).is_some_and(|class| class & 0x2f == 0x2f)
            || !diag::rd::<i32>(p + OFF_ACTIVE).is_some_and(|active| active != 0)
            || !diag::rd::<i32>(p + OFF_STATE).is_some_and(|state| !matches!(state, 6 | 7))
            || !diag::rd::<f32>(p + OFF_SIDE).is_some_and(|side| side < 0.0)
        {
            continue;
        }
        let d =
            (read::<f32>(p + OFF_POS_X) - fx).powi(2) + (read::<f32>(p + OFF_POS_Y) - fy).powi(2);
        if d < best.0 {
            best = (d, p);
        }
    }
    best.1
}

/// While an ally is being tracked, one `ALLY track` line per game second for 60 s: where it is, where it
/// wants to go, what it targets, and where the player is.
pub(crate) unsafe fn tick(ships: &[usize]) {
    let ally = TRACK_ALLY.load(Ordering::Relaxed);
    if ally == 0 || !diag::debug() {
        return;
    }
    let now = read::<f64>(CLOCK);
    if !ships.contains(&ally) || now > f64::from_bits(TRACK_END.load(Ordering::Relaxed)) {
        log::info!(
            "ALLY track ended (still in roster: {})",
            ships.contains(&ally)
        );
        TRACK_ALLY.store(0, Ordering::Relaxed);
        return;
    }
    let Some(creatures) = probe_creatures() else {
        return;
    };
    // every frame: count launches and shots of the ally and of its nearest enemy
    if !probe_ship_in_battle(ally, &creatures) {
        return;
    }
    if let (Ok(mut a), Ok(mut e)) = (PROBE_ALLY.lock(), PROBE_ENEMY.lock()) {
        if a.who != ally {
            a.reset(ally, now);
            e.reset(0, now);
        }
        // Check membership first: never read an old enemy's fields to decide
        // whether that pointer still belongs to an enemy ship.
        if !probe_ship_in_battle(e.who, &creatures)
            || !diag::rd::<f32>(e.who + OFF_SIDE).is_some_and(|side| side < 0.0)
        {
            let enemy = nearest_enemy(ally, &creatures);
            e.reset(enemy, now);
        }
        a.count();
        e.count();
        if now >= f64::from_bits(TRACK_NEXT.load(Ordering::Relaxed)) {
            log::info!("{}", a.line("ally ", now, &creatures));
            if e.who != 0 {
                log::info!("{}", e.line("enemy", now, &creatures));
            }
        }
    }
    if now < f64::from_bits(TRACK_NEXT.load(Ordering::Relaxed)) {
        return;
    }
    TRACK_NEXT.store((now + 1.0).to_bits(), Ordering::Relaxed);
    let player = ships
        .iter()
        .copied()
        .find(|&p| read::<i32>(p + OFF_MODE_D70) == 1);
    let (px, py) = player.map_or((0.0, 0.0), |p| {
        (read::<f32>(p + OFF_POS_X), read::<f32>(p + OFF_POS_Y))
    });
    let (ax, ay) = (read::<f32>(ally + OFF_POS_X), read::<f32>(ally + OFF_POS_Y));
    log::info!(
        "ALLY track t={now:.1} ally=({ax:.0},{ay:.0}) want=({:.0},{:.0}) player=({px:.0},{py:.0}) dist={:.0} state={} target={:#x} ai_flag={} mode={}",
        read::<f32>(ally + OFF_WAYPOINT_X),
        read::<f32>(ally + OFF_WAYPOINT_Y),
        ((ax - px).powi(2) + (ay - py).powi(2)).sqrt(),
        read::<i32>(ally + OFF_STATE),
        read::<usize>(ally + OFF_TARGET),
        read::<u8>(ally + OFF_AI_FLAG),
        read::<i32>(ally + OFF_MODE_D70),
    );
}
