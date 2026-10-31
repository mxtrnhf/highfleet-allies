//! The pure part of the `tactics` stage: aim dodging, dive avoidance,
//! steering composition, eligibility gates and shared safety clamps. No game memory, no serde, no `crate::`, so it
//! can be tested on the host alone: `rustc --edition 2021 --test src/tactics_math.rs -o /tmp/tm_test && /tmp/tm_test`.
//!
//! Coordinates are the game's: **y points down** (landed ships sit at y +165..+400, airborne fights run up to y -2000).
//! "Down" is therefore +y, the ground is at large +y, and "up" is -y.

/// Length of the waypoint vector, exactly like the stock AI's (it is not known whether the length feeds thrust).
pub const WAYPOINT_LEN: f32 = 200.0;
/// A blended direction shorter than this (stock unit vector + weighted tactic) means the two nearly cancel: stock.
pub const MIN_BLEND_LEN: f32 = 0.2;
/// The game clock moving forward by more than this between two calls (or backwards at all) means a new battle.
pub const CLOCK_JUMP: f64 = 5.0;
/// Entries not seen for this long are dropped.
pub const STALE_AFTER: f64 = 2.0;
/// Tolerance of the final constraint check.
const EPS: f32 = 1.0e-3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// No reactive dodge applies, or an eligibility gate failed; leave stock steering.
    Stock,
    /// The ported missile panic owns the waypoint (set by the caller).
    Panic,
    /// "Deny the dive": sidestepping out of the line straight above / below the target.
    Dive,
    /// The target sat in the dive cone for the dwell time but the chance roll said no (stays stock until it leaves).
    DiveSkip,
    /// "Aim awareness": the player's aim line crosses this ship during an awareness window, it slips off the line.
    Aim,
}

/// Sanitized config values the tactics use.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub enabled: bool,
    pub apply_to_allies: bool,
    /// Ground reference in game y (y grows downward). The ground itself varies with x; see `ground_clearance`.
    pub ground_y: f32,
    /// Closer than this above `ground_y` (ship.y > ground_y - ground_clearance) no downward component is added.
    pub ground_clearance: f32,
    /// Below this y (ship.y larger = lower; the ground is about +250) no tactic may steer downward at all, whatever the
    /// stock direction does (enemies were seen following the player into the ground).
    pub ground_hard_y: f32,
    /// No outward component within this distance of an arena edge.
    pub edge_margin: f32,
    /// "Deny the dive": when the target hangs straight below / above for `dive_dwell_s`, sidestep for a while.
    pub dive_enabled: bool,
    /// Chance the dodge starts, rolled once per time the target enters the cone.
    pub dive_chance: f32,
    /// Half-angle (degrees) around straight down / straight up that counts as "under / over".
    pub dive_cone_deg: f32,
    /// Only counts within this distance.
    pub dive_range: f32,
    /// The target must stay in the cone this long before the roll.
    pub dive_dwell_s: f32,
    /// The dodge lasts a random time in this range.
    pub dive_min_s: f32,
    pub dive_max_s: f32,
    /// After a dodge ends, no new dodge for this long.
    pub dive_cooldown_s: f32,
    /// Weight of the dive sidestep against the unit stock direction.
    pub dive_weight: f32,
    /// React to a target below us ("under") and / or above us ("over").
    pub dive_under: bool,
    pub dive_over: bool,
    /// Engine boost factor while dodging (1.0 = none; the player's Shift is 2.0).
    pub dive_boost: f32,
    /// Wrong-foot: when the target moves sideways faster than this (u/s), the dodge goes the opposite way (0 = off).
    pub dive_wrongfoot_speed: f32,
    /// The target BELOW us ("under", the damaging case) has its own chance and dwell; `dive_chance` / `dive_dwell_s`
    /// apply to a target above us.
    pub dive_under_chance: f32,
    pub dive_under_dwell_s: f32,
    /// Share of vertical motion towards the target's altitude mixed into the dodge (0 = purely sideways): the ship
    /// tries to get onto the target's level so it cannot be shot in the belly.
    pub dive_level_weight: f32,
    /// "Aim awareness": during shared awareness windows the enemies that target the player see where the player aims
    /// and slip off the aim line.
    pub aim_enabled: bool,
    /// Length of one awareness window and the random time from the START of one window to the start of the next.
    pub aim_window_s: f32,
    pub aim_gap_min_s: f32,
    pub aim_gap_max_s: f32,
    /// A ship counts as on the line when its miss distance from the aim ray is below this.
    pub aim_tolerance: f32,
    /// ... and it is not farther than this along the ray.
    pub aim_range: f32,
    pub aim_weight: f32,
    pub aim_boost: f32,
    /// The dodge only happens while the player is actually shooting (fire byte of either gun group set).
    pub aim_require_fire: bool,
    /// Early fire: once per awareness window, a ship that is on the aim line rolls this chance to open fire with its
    /// main guns (group 0) for `aim_fire_s`, as long as the player is within `aim_fire_range` x its gun reach.
    pub aim_fire_probability: f32,
    pub aim_fire_s: f32,
    pub aim_fire_range: f32,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            enabled: true,
            apply_to_allies: true,
            ground_y: 200.0,
            ground_clearance: 450.0,
            ground_hard_y: 120.0,
            edge_margin: 300.0,
            dive_enabled: true,
            dive_chance: 0.8,
            dive_cone_deg: 45.0,
            dive_range: 600.0,
            dive_dwell_s: 1.0,
            dive_min_s: 1.5,
            dive_max_s: 3.0,
            dive_cooldown_s: 2.0,
            dive_weight: 8.0,
            dive_under: true,
            dive_over: true,
            dive_boost: 2.5,
            dive_wrongfoot_speed: 40.0,
            dive_under_chance: 1.0,
            dive_under_dwell_s: 0.5,
            dive_level_weight: 0.7,
            aim_enabled: true,
            aim_window_s: 1.5,
            aim_gap_min_s: 2.5,
            aim_gap_max_s: 3.5,
            aim_tolerance: 200.0,
            aim_range: 1800.0,
            aim_weight: 12.0,
            aim_boost: 2.5,
            aim_require_fire: true,
            aim_fire_probability: 0.5,
            aim_fire_s: 1.0,
            aim_fire_range: 1.6,
        }
    }
}

/// Arena extents (game x min / max, y min = top). The bottom is not used (terrain is the floor).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub xmin: f32,
    pub xmax: f32,
    pub ymin: f32,
}

impl Bounds {
    /// Validates the four floats the game keeps at `0x1403c2bc0..` (`[0]` x max, `[1]` y max, `[2]` x min, `[3]` y min).
    pub fn from_raw(raw: [f32; 4]) -> Option<Bounds> {
        let [xmax, ymax, xmin, ymin] = raw;
        if !raw.iter().all(|v| v.is_finite() && v.abs() <= 20_000.0) {
            return None;
        }
        if xmin >= xmax || xmax - xmin < 600.0 || ymin >= ymax || ymin > 0.0 {
            return None;
        }
        Some(Bounds { xmin, xmax, ymin })
    }
}

/// xorshift64* with its own state (no globals), seeded per ship.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mixed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
        Rng(if mixed == 0 { 0xdead_beef_cafe_1234 } else { mixed })
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut s = self.0;
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        self.0 = s;
        s.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo).max(0.0) * self.unit()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Pass,
    /// Some gate failed: stock behaviour.
    Stock,
    /// The ship is in a missile panic: the panic stage owns the waypoint.
    Panic,
}

/// Everything the gate looks at (already read from the game).
#[derive(Clone, Copy, Debug)]
pub struct GateInput {
    pub ai_flag: u8,
    pub side: f32,
    pub mode: Option<i32>,
    pub has_target: bool,
    pub panicking: bool,
}

/// `Pass` only when every gate holds. Mode 1 = airborne combat (`+0xb48`; landed is 3, takeoff 0).
pub fn gate(p: &Params, g: &GateInput) -> Gate {
    if !p.enabled || g.ai_flag == 0 || !g.side.is_finite() || g.side == 0.0 {
        return Gate::Stock;
    }
    if g.side > 0.0 && !p.apply_to_allies {
        return Gate::Stock;
    }
    if g.mode != Some(1) || !g.has_target {
        return Gate::Stock;
    }
    if g.panicking {
        return Gate::Panic;
    }
    Gate::Pass
}

/// Larger of the two group reaches that is positive and finite.
pub fn best_reach(a: f32, b: f32) -> Option<f32> {
    [a, b].into_iter().filter(|r| r.is_finite() && *r > 0.0).fold(None, |m, r| Some(m.map_or(r, |m: f32| m.max(r))))
}

pub fn normalized(x: f32, y: f32) -> Option<(f32, f32)> {
    let len = (x * x + y * y).sqrt();
    (x.is_finite() && y.is_finite() && len.is_finite() && len > 1.0e-3).then(|| (x / len, y / len))
}

/// Per-ship tactic state.
#[derive(Clone, Debug)]
pub struct ShipState {
    pub last_seen: f64,
    /// Largest own reach seen (0 = none yet).
    pub reach_self: f32,
    /// Target in the dive cone since this time.
    pub aligned_since: Option<f64>,
    /// The roll for the current alignment said no.
    pub declined: bool,
    /// A dodge in progress: (until, side) with side +1 = towards +x.
    pub dive: Option<(f64, f32)>,
    pub dive_cooldown_until: f64,
    /// Last target position and time, and its smoothed sideways velocity (u/s).
    pub prev_target: Option<(f64, (f32, f32))>,
    pub target_vx: f32,
}

impl ShipState {
    pub fn new(now: f64) -> Self {
        Self {
            last_seen: now, reach_self: 0.0, aligned_since: None, declined: false,
            dive: None, dive_cooldown_until: 0.0, prev_target: None, target_vx: 0.0,
        }
    }

    /// Keep the largest own gun reach seen for early fire; AP can lower the live value.
    pub fn latch_reach(&mut self, own: Option<f32>) {
        if let Some(reach) = own.filter(|r| r.is_finite() && *r > 0.0) {
            self.reach_self = self.reach_self.max(reach);
        }
    }

    pub fn reach(&self) -> Option<f32> {
        (self.reach_self > 0.0).then_some(self.reach_self)
    }

}

/// Whether the per-ship table must be cleared: the clock went backwards or jumped (a new battle).
pub fn clock_reset(last: f64, now: f64) -> bool {
    !now.is_finite() || now < last || now - last > CLOCK_JUMP
}

/// Makes `d[axis] <= limit` by turning the unit vector (the other component keeps its sign).
fn cap_upper(d: &mut (f32, f32), axis: usize, limit: f32) {
    clamp_axis(d, axis, limit, true);
}
/// Makes `d[axis] >= limit`.
fn cap_lower(d: &mut (f32, f32), axis: usize, limit: f32) {
    clamp_axis(d, axis, limit, false);
}

fn clamp_axis(d: &mut (f32, f32), axis: usize, limit: f32, upper: bool) {
    let get = |d: &(f32, f32), i: usize| if i == 0 { d.0 } else { d.1 };
    let v = get(d, axis);
    if (upper && v <= limit) || (!upper && v >= limit) {
        return;
    }
    let lim = limit.clamp(-1.0, 1.0);
    let other = get(d, 1 - axis);
    let sign = if other < 0.0 { -1.0 } else { 1.0 };
    let rest = (1.0 - lim * lim).max(0.0).sqrt() * sign;
    *d = if axis == 0 { (lim, rest) } else { (rest, lim) };
}

/// The constraints implied by the ship position and the stock direction: `dir.y <= y_up_limit` etc.
struct Limits {
    max_y: Option<f32>,
    max_x: Option<f32>,
    min_x: Option<f32>,
    min_y: Option<f32>,
}

fn limits(ship: (f32, f32), stock: (f32, f32), bounds: &Bounds, p: &Params) -> Limits {
    // Never more downward (+y) than the stock direction, and never downward at all if stock is level or up.
    let max_y = if ship.1 > p.ground_hard_y {
        Some(0.0)
    } else {
        (ship.1 > p.ground_y - p.ground_clearance).then(|| stock.1.max(0.0))
    };
    let max_x = (ship.0 > bounds.xmax - p.edge_margin).then(|| stock.0.max(0.0));
    let min_x = (ship.0 < bounds.xmin + p.edge_margin).then(|| stock.0.min(0.0));
    let min_y = (ship.1 < bounds.ymin + p.edge_margin).then(|| stock.1.min(0.0));
    Limits { max_y, max_x, min_x, min_y }
}

fn satisfies(d: (f32, f32), l: &Limits) -> bool {
    l.max_y.map_or(true, |m| d.1 <= m + EPS)
        && l.max_x.map_or(true, |m| d.0 <= m + EPS)
        && l.min_x.map_or(true, |m| d.0 >= m - EPS)
        && l.min_y.map_or(true, |m| d.1 >= m - EPS)
}

/// `ship + 200 * normalize(stock_unit + w * tactic)`, with the safety limits applied to the FINAL direction (the
/// normalization of a partly opposing sum can be more downward or outward than either input). `None` when the two
/// nearly cancel, when a limit cannot be met (corner conflicts) or on any non-finite value: the caller leaves stock.
pub fn compose(ship: (f32, f32), stock_vec: (f32, f32), tactic: (f32, f32), w: f32, bounds: &Bounds, p: &Params) -> Option<(f32, f32)> {
    if !(ship.0.is_finite() && ship.1.is_finite() && tactic.0.is_finite() && tactic.1.is_finite() && w.is_finite()) {
        return None;
    }
    let stock = normalized(stock_vec.0, stock_vec.1).unwrap_or((0.0, 0.0));
    let sum = (stock.0 + w * tactic.0, stock.1 + w * tactic.1);
    let len = (sum.0 * sum.0 + sum.1 * sum.1).sqrt();
    if !len.is_finite() || len < MIN_BLEND_LEN {
        return None;
    }
    let mut dir = (sum.0 / len, sum.1 / len);
    let l = limits(ship, stock, bounds, p);
    if let Some(m) = l.max_y {
        cap_upper(&mut dir, 1, m);
    }
    if let Some(m) = l.min_y {
        cap_lower(&mut dir, 1, m);
    }
    if let Some(m) = l.max_x {
        cap_upper(&mut dir, 0, m);
    }
    if let Some(m) = l.min_x {
        cap_lower(&mut dir, 0, m);
    }
    let dir = normalized(dir.0, dir.1)?;
    if !satisfies(dir, &l) {
        return None;
    }
    let out = (ship.0 + WAYPOINT_LEN * dir.0, ship.1 + WAYPOINT_LEN * dir.1);
    (out.0.is_finite() && out.1.is_finite()).then_some(out)
}

/// Is the target within the cone straight below / above, within range? `widen` > 1 loosens it (hysteresis while already aligned).
pub fn in_dive_cone(to: (f32, f32), p: &Params, widen: f32) -> bool {
    let d = (to.0 * to.0 + to.1 * to.1).sqrt();
    if !(d.is_finite() && d > 1.0 && d <= p.dive_range * widen) {
        return false;
    }
    let cone = (p.dive_cone_deg * widen).clamp(1.0, 89.0).to_radians();
    let below = to.1 > 0.0;
    ((below && p.dive_under) || (!below && p.dive_over)) && to.0.abs() / d <= cone.sin()
}

/// Smooths the target's sideways velocity from successive positions (a gap outside 20..500 ms restarts it).
pub fn track_target(st: &mut ShipState, now: f64, target: (f32, f32)) {
    if let Some((t0, p0)) = st.prev_target {
        let dt = now - t0;
        if dt > 0.02 && dt < 0.5 {
            let inst = (target.0 - p0.0) / dt as f32;
            if inst.is_finite() {
                st.target_vx = if st.target_vx == 0.0 { inst } else { 0.8 * st.target_vx + 0.2 * inst };
            }
        } else if dt >= 0.5 || dt < 0.0 {
            st.target_vx = 0.0;
        } else {
            return; // too soon: keep the older sample as the reference
        }
    }
    st.prev_target = Some((now, target));
}

/// Side of the sidestep: away from a near arena wall; else AGAINST the target's sideways motion (it has to reverse
/// its momentum to follow: "wrong-foot"); else random.
pub fn dive_side(ship_x: f32, bounds: &Bounds, p: &Params, rng: &mut Rng, target_vx: f32) -> f32 {
    let wall = 2.0 * p.edge_margin;
    if bounds.xmax - ship_x < wall && ship_x - bounds.xmin >= wall {
        -1.0
    } else if ship_x - bounds.xmin < wall && bounds.xmax - ship_x >= wall {
        1.0
    } else if p.dive_wrongfoot_speed > 0.0 && target_vx.is_finite() && target_vx.abs() >= p.dive_wrongfoot_speed {
        -target_vx.signum()
    } else if rng.unit() < 0.5 {
        -1.0
    } else {
        1.0
    }
}

enum Dive {
    /// Nothing to do.
    Idle,
    /// Declined by the roll.
    Skip,
    /// Dodging: the sidestep direction.
    Active(f32),
}

fn dive_update(p: &Params, st: &mut ShipState, bounds: &Bounds, now: f64, ship: (f32, f32), to: (f32, f32), rng: &mut Rng) -> Dive {
    if let Some((until, side)) = st.dive {
        if now < until {
            return Dive::Active(side);
        }
        st.dive = None;
        st.dive_cooldown_until = now + f64::from(p.dive_cooldown_s);
        st.aligned_since = None;
        st.declined = false;
    }
    if now < st.dive_cooldown_until {
        return Dive::Idle;
    }
    let widen = if st.aligned_since.is_some() { 1.15 } else { 1.0 };
    if !in_dive_cone(to, p, widen) {
        st.aligned_since = None;
        st.declined = false;
        return Dive::Idle;
    }
    let since = *st.aligned_since.get_or_insert(now);
    if st.declined {
        return Dive::Skip;
    }
    let under = to.1 > 0.0;
    let (dwell, chance) = if under { (p.dive_under_dwell_s, p.dive_under_chance) } else { (p.dive_dwell_s, p.dive_chance) };
    if now - since < f64::from(dwell) {
        return Dive::Idle;
    }
    if rng.unit() < chance {
        let side = dive_side(ship.0, bounds, p, rng, st.target_vx);
        st.dive = Some((now + f64::from(rng.range(p.dive_min_s, p.dive_max_s)), side));
        Dive::Active(side)
    } else {
        st.declined = true;
        Dive::Skip
    }
}

/// The waypoint kept inside the arena sides and ceiling (50 u margin), per axis: a waypoint beyond a border is moved
/// onto it (a ship already outside is pulled back in rather than left with a zero-length course).
pub fn clamp_waypoint(wp: (f32, f32), ship: (f32, f32), b: &Bounds) -> (f32, f32) {
    const MARGIN: f32 = 50.0;
    if !(wp.0.is_finite() && wp.1.is_finite() && ship.0.is_finite() && ship.1.is_finite()) {
        return wp;
    }
    let x = wp.0.clamp(b.xmin + MARGIN, b.xmax - MARGIN);
    let y = wp.1.max(b.ymin + MARGIN);
    if (x - ship.0).hypot(y - ship.1) < 1.0 {
        return wp; // clamping would leave no course at all
    }
    (x, y)
}

/// The shared awareness clock: windows of `aim_window_s` opening at random intervals.
#[derive(Clone, Debug, Default)]
pub struct AimClock {
    pub open_until: f64,
    pub next_open: f64,
    pub started: bool,
    /// How many windows have opened (a ship rolls its early fire once per window number).
    pub window: u32,
}

impl AimClock {
    /// Advances the clock to `now` and says whether a window is open.
    pub fn update(&mut self, now: f64, p: &Params, rng: &mut Rng) -> bool {
        if !self.started || now < self.open_until - f64::from(p.aim_window_s) - 1.0 {
            // first call, or the game clock restarted
            self.started = true;
            self.open_until = 0.0;
            self.next_open = now + f64::from(rng.range(p.aim_gap_min_s, p.aim_gap_max_s));
            return false;
        }
        if now >= self.next_open && now >= self.open_until {
            self.open_until = now + f64::from(p.aim_window_s);
            self.window = self.window.wrapping_add(1);
            // gap = start to start, never shorter than the window itself
            self.next_open = now + f64::from(rng.range(p.aim_gap_min_s, p.aim_gap_max_s)).max(f64::from(p.aim_window_s));
        }
        now < self.open_until
    }
}

/// Early fire is allowed while the player is inside `aim_fire_range` x the ship's gun reach (reach unknown: never).
pub fn early_fire_in_range(dist: f32, reach: Option<f32>, p: &Params) -> bool {
    reach.is_some_and(|r| r.is_finite() && r > 0.0 && dist.is_finite() && dist <= r * p.aim_fire_range)
}

/// The player's aim as the AI would "see" it: where the player is and the unit direction of the aim.
#[derive(Clone, Copy, Debug)]
pub struct AimInput {
    pub player: (f32, f32),
    pub dir: (f32, f32),
}

/// Is `ship` on the aim ray? Returns the unit direction that slips off the line (perpendicular to the aim, on the side
/// the ship already is). `None` when the ship is behind the player, too far along the ray, or off the line.
pub fn aim_dodge_dir(aim: &AimInput, ship: (f32, f32), p: &Params, rng: &mut Rng) -> Option<(f32, f32)> {
    let v = (ship.0 - aim.player.0, ship.1 - aim.player.1);
    let along = v.0 * aim.dir.0 + v.1 * aim.dir.1;
    let cross = aim.dir.0 * v.1 - aim.dir.1 * v.0;
    if !(along.is_finite() && cross.is_finite()) || along <= 0.0 || along > p.aim_range || cross.abs() > p.aim_tolerance {
        return None;
    }
    let sign = if cross.abs() < 1.0 {
        if rng.unit() < 0.5 { -1.0 } else { 1.0 }
    } else {
        cross.signum()
    };
    // the normal n = (-dir.y, dir.x) has n . v = cross, so the ship sits on the side sign(cross) of the line
    normalized(-aim.dir.1 * sign, aim.dir.0 * sign)
}

pub struct StepInput {
    pub now: f64,
    pub aim: Option<AimInput>,
    pub ship: (f32, f32),
    pub wanted: (f32, f32),
    pub target: (f32, f32),
}

#[derive(Clone, Copy, Debug)]
pub struct Step {
    pub state: State,
    pub waypoint: Option<(f32, f32)>,
}

/// One decision for a ship that passed the gates. Updates the side timer and the state.
pub fn step(p: &Params, st: &mut ShipState, bounds: &Bounds, inp: &StepInput, rng: &mut Rng) -> Step {
    st.last_seen = inp.now;
    track_target(st, inp.now, inp.target);
    let to = (inp.target.0 - inp.ship.0, inp.target.1 - inp.ship.1);
    let stock_vec = (inp.wanted.0 - inp.ship.0, inp.wanted.1 - inp.ship.1);
    let mut skipped = false;
    if let (true, Some(aim)) = (p.aim_enabled, inp.aim) {
        if let Some(dir) = aim_dodge_dir(&aim, inp.ship, p, rng) {
            let waypoint = compose(inp.ship, stock_vec, dir, p.aim_weight, bounds, p);
            return Step { state: State::Aim, waypoint };
        }
    }
    if p.dive_enabled {
        match dive_update(p, st, bounds, inp.now, inp.ship, to, rng) {
            Dive::Active(side) => {
                // sideways, plus a pull onto the target's altitude (smooth: full strength from 150 u of height difference)
                let level = (to.1 / 150.0).clamp(-1.0, 1.0) * p.dive_level_weight;
                let dir = normalized(side, level).unwrap_or((side, 0.0));
                let waypoint = compose(inp.ship, stock_vec, dir, p.dive_weight, bounds, p);
                return Step { state: State::Dive, waypoint };
            }
            Dive::Skip => skipped = true,
            Dive::Idle => {}
        }
    }
    Step { state: if skipped { State::DiveSkip } else { State::Stock }, waypoint: None }
}
