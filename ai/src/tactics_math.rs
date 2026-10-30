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

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Stock => "stock",
            State::Panic => "panic",
            State::Dive => "dive",
            State::DiveSkip => "dive_skip",
            State::Aim => "aim",
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Bounds {
        Bounds { xmin: -1500.0, xmax: 1500.0, ymin: -2000.0 }
    }
    fn on() -> Params {
        Params { enabled: true, apply_to_allies: false, ..Params::default() }
    }
    fn ok_gate() -> GateInput {
        GateInput { ai_flag: 1, side: -1.0, mode: Some(1), has_target: true, panicking: false }
    }
    fn len(a: (f32, f32), b: (f32, f32)) -> f32 {
        ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
    }

    #[test]
    fn no_dodge_leaves_stock_steering_at_every_former_band_distance() {
        let params = on();
        let mut state = ShipState::new(0.0);
        let mut rng = Rng::new(3);
        for distance in [100.0, 250.0, 480.0, 580.0, 900.0, 2000.0] {
            let input = StepInput {
                now: 1.0, aim: None, ship: (0.0, -1000.0),
                wanted: (200.0, -1000.0), target: (distance, -1000.0),
            };
            let decision = step(&params, &mut state, &bounds(), &input, &mut rng);
            assert_eq!((decision.state, decision.waypoint), (State::Stock, None));
        }
    }

    #[test]
    fn aim_dodge_still_preempts_dive_and_dive_works_without_aim() {
        let input = StepInput {
            now: 1.0, ship: (0.0, -1000.0), wanted: (200.0, -1000.0),
            target: (0.0, -700.0),
            aim: Some(AimInput { player: (0.0, -700.0), dir: (0.0, -1.0) }),
        };
        for (aim_enabled, expected) in [(true, State::Aim), (false, State::Dive)] {
            let params = Params {
                enabled: true, aim_enabled, dive_under_dwell_s: 0.0,
                dive_under_chance: 1.0, ..Params::default()
            };
            let mut state = ShipState::new(0.0);
            let mut rng = Rng::new(3);
            let decision = step(&params, &mut state, &bounds(), &input, &mut rng);
            assert_eq!(decision.state, expected);
            assert!(decision.waypoint.is_some());
        }
    }

    #[test]
    fn gate_combinations() {
        let p = on();
        assert_eq!(gate(&p, &ok_gate()), Gate::Pass);
        assert_eq!(gate(&Params { enabled: false, ..Params::default() }, &ok_gate()), Gate::Stock, "disabled");
        assert_eq!(gate(&p, &GateInput { ai_flag: 0, ..ok_gate() }), Gate::Stock, "player-flown");
        assert_eq!(gate(&p, &GateInput { side: 1.0, ..ok_gate() }), Gate::Stock, "ally");
        assert_eq!(gate(&Params { apply_to_allies: true, ..p }, &GateInput { side: 1.0, ..ok_gate() }), Gate::Pass);
        assert_eq!(gate(&p, &GateInput { side: 0.0, ..ok_gate() }), Gate::Stock);
        assert_eq!(gate(&p, &GateInput { side: f32::NAN, ..ok_gate() }), Gate::Stock);
        for mode in [0, 3, 2, -1] {
            assert_eq!(gate(&p, &GateInput { mode: Some(mode), ..ok_gate() }), Gate::Stock, "mode {mode}");
        }
        assert_eq!(gate(&p, &GateInput { mode: None, ..ok_gate() }), Gate::Stock);
        assert_eq!(gate(&p, &GateInput { has_target: false, ..ok_gate() }), Gate::Stock);
        assert_eq!(gate(&p, &GateInput { panicking: true, ..ok_gate() }), Gate::Panic);
        assert_eq!(gate(&p, &GateInput { panicking: true, mode: Some(3), ..ok_gate() }), Gate::Stock);
    }


    #[test]
    fn ap_reach_latch_preserves_early_fire_range() {
        let mut state = ShipState::new(0.0);
        assert_eq!(state.reach(), None);
        state.latch_reach(Some(1500.0));
        state.latch_reach(Some(500.0));
        state.latch_reach(None);
        assert_eq!(state.reach(), Some(1500.0));
        assert!(early_fire_in_range(2000.0, state.reach(), &Params::default()));
        assert_eq!(best_reach(0.0, 1500.0), Some(1500.0));
        assert_eq!(best_reach(f32::NAN, -1.0), None);
    }




    #[test]
    fn compose_length_is_exactly_200() {
        let p = on();
        for (stock, tac, w) in [((200.0, 0.0), (0.0, 1.0), 1.0), ((-50.0, 30.0), (0.6, -0.8), 0.5), ((1.0, 1.0), (1.0, 0.0), 2.0)] {
            let ship = (100.0, -800.0);
            let out = compose(ship, stock, tac, w, &bounds(), &p).unwrap();
            assert!((len(out, ship) - 200.0).abs() < 1e-3, "{out:?}");
        }
    }

    #[test]
    fn compose_fallbacks_never_nan() {
        let p = on();
        let ship = (0.0, -800.0);
        // stock zero vector: tactic alone
        let out = compose(ship, (0.0, 0.0), (0.0, -1.0), 1.0, &bounds(), &p).unwrap();
        assert!((len(out, ship) - 200.0).abs() < 1e-3);
        // stock zero and weight zero: nothing left
        assert!(compose(ship, (0.0, 0.0), (0.0, -1.0), 0.0, &bounds(), &p).is_none());
        // exact cancel
        assert!(compose(ship, (100.0, 0.0), (-1.0, 0.0), 1.0, &bounds(), &p).is_none());
        // near cancel
        assert!(compose(ship, (100.0, 0.0), (-0.9, 0.0), 1.0, &bounds(), &p).is_none());
        for bad in [f32::NAN, f32::INFINITY] {
            assert!(compose(ship, (bad, 0.0), (0.0, 1.0), 1.0, &bounds(), &p).map_or(true, |o| o.0.is_finite() && o.1.is_finite()));
            assert!(compose(ship, (1.0, 0.0), (bad, 1.0), 1.0, &bounds(), &p).is_none());
            assert!(compose((bad, 0.0), (1.0, 0.0), (0.0, 1.0), 1.0, &bounds(), &p).is_none());
            assert!(compose(ship, (1.0, 0.0), (0.0, 1.0), bad, &bounds(), &p).is_none());
        }
        assert!(normalized(0.0, 0.0).is_none());
        assert!(normalized(f32::NAN, 1.0).is_none());
    }

    #[test]
    fn ground_rule_y_points_down() {
        let p = on();
        // ship at y = +100 (close to the ground, +y is down): a downward tactic must not survive
        let ship = (0.0, 100.0);
        let out = compose(ship, (200.0, 0.0), (0.0, 1.0), 1.0, &bounds(), &p).unwrap();
        assert!(out.1 <= ship.1 + 1e-3, "no downward component near the ground: {out:?}");
        // an upward tactic (-y) is left alone
        let up = compose(ship, (200.0, 0.0), (0.0, -1.0), 1.0, &bounds(), &p).unwrap();
        assert!(up.1 < ship.1 - 50.0);
        // high above the ground (y = -1200): downward is fine
        let high = (0.0, -1200.0);
        let dn = compose(high, (200.0, 0.0), (0.0, 1.0), 1.0, &bounds(), &p).unwrap();
        assert!(dn.1 > high.1 + 50.0);
        // zone edge: ground_y - clearance = -250
        let edge_in = compose((0.0, -240.0), (200.0, 0.0), (0.0, 1.0), 1.0, &bounds(), &p).unwrap();
        assert!(edge_in.1 <= -240.0 + 1e-3);
        let edge_out = compose((0.0, -260.0), (200.0, 0.0), (0.0, 1.0), 1.0, &bounds(), &p).unwrap();
        assert!(edge_out.1 > -260.0 + 50.0);
    }

    #[test]
    fn opposing_blend_cannot_become_more_downward_than_stock_or_level() {
        // the advisor's example: stock (0.6, 0.8) (down) + tactic (-1, 0) normalises to (-0.45, 0.89), more downward
        // than the stock's own y share; near the ground the final y must not exceed max(stock.y, 0) = 0.8
        let p = on();
        let ship = (0.0, 100.0);
        let stock = (0.6 * 200.0, 0.8 * 200.0);
        let out = compose(ship, stock, (-1.0, 0.0), 1.0, &bounds(), &p).unwrap();
        let dir_y = (out.1 - ship.1) / 200.0;
        assert!(dir_y <= 0.8 + 1e-3, "{dir_y}");
        // and with an upward stock the final direction must be level or up
        let out = compose(ship, (0.0, -200.0), (-1.0, 0.0), 2.0, &bounds(), &p).unwrap();
        assert!(out.1 <= ship.1 + 1e-3);
        // stock level, tactic partly down: clamp to level
        let out = compose(ship, (200.0, 0.0), (-0.3, 0.95), 1.0, &bounds(), &p).unwrap();
        assert!(out.1 <= ship.1 + 1e-3);
        assert!((len(out, ship) - 200.0).abs() < 1e-3);
    }

    #[test]
    fn arena_edges_no_outward_push() {
        let p = on();
        let b = bounds();
        // near the right edge (x max 1500, margin 300): no +x
        let ship = (1350.0, -1000.0);
        let out = compose(ship, (0.0, 200.0), (1.0, 0.0), 1.0, &b, &p).unwrap();
        assert!(out.0 <= ship.0 + 1e-3);
        // inward is fine
        let inward = compose(ship, (0.0, 200.0), (-1.0, 0.0), 1.0, &b, &p).unwrap();
        assert!(inward.0 < ship.0 - 50.0);
        // left edge
        let ship = (-1350.0, -1000.0);
        let out = compose(ship, (0.0, 200.0), (-1.0, 0.0), 1.0, &b, &p).unwrap();
        assert!(out.0 >= ship.0 - 1e-3);
        // top (y min -2000, more negative = outward): no -y
        let ship = (0.0, -1850.0);
        let out = compose(ship, (200.0, 0.0), (0.0, -1.0), 1.0, &b, &p).unwrap();
        assert!(out.1 >= ship.1 - 1e-3);
        let down = compose(ship, (200.0, 0.0), (0.0, 1.0), 1.0, &b, &p).unwrap();
        assert!(down.1 > ship.1 + 50.0);
        // stock itself heading out is not fought: the limit is the stock's own component
        let ship = (1350.0, -1000.0);
        let out = compose(ship, (200.0, 0.0), (0.0, 1.0), 1.0, &b, &p).unwrap();
        assert!(out.0 <= ship.0 + 200.0 + 1e-3);
    }

    #[test]
    fn corner_conflict_returns_none_or_satisfies() {
        let p = on();
        let b = bounds();
        // bottom-right: near ground and near right edge, tactic pushes down-right
        let ship = (1400.0, 100.0);
        if let Some(o) = compose(ship, (0.0, 200.0), (1.0, 1.0), 1.0, &b, &p) {
            assert!(o.0 <= ship.0 + 1e-3 && o.1 <= ship.1 + 200.0 + 1e-3);
        }
        let ship = (1400.0, 100.0);
        if let Some(o) = compose(ship, (-200.0, 0.0), (1.0, 1.0), 1.0, &b, &p) {
            assert!(o.0 <= ship.0 + 1e-3 && o.1 <= ship.1 + 1e-3);
        }
    }

    #[test]
    fn bounds_validation() {
        assert!(Bounds::from_raw([1500.0, 400.0, -1500.0, -2000.0]).is_some());
        assert!(Bounds::from_raw([6000.0, 6000.0, -6000.0, -6000.0]).is_some());
        assert!(Bounds::from_raw([0.0; 4]).is_none());
        assert!(Bounds::from_raw([-1.0, 400.0, 1.0, -2000.0]).is_none());
        assert!(Bounds::from_raw([f32::NAN, 400.0, -1500.0, -2000.0]).is_none());
        assert!(Bounds::from_raw([50_000.0, 400.0, -1500.0, -2000.0]).is_none());
        assert!(Bounds::from_raw([200.0, 400.0, -200.0, -2000.0]).is_none(), "too narrow");
    }


    #[test]
    fn rng_never_sticks_at_zero() {
        let mut r = Rng::new(0);
        let v: Vec<u64> = (0..4).map(|_| r.next_u64()).collect();
        assert!(v.iter().all(|x| *x != 0) && v[0] != v[1]);
        let mut r = Rng::new(5);
        for _ in 0..1000 {
            let u = r.unit();
            assert!((0.0..1.0).contains(&u));
        }
    }

    #[test]
    fn clock_jump_and_backwards_reset() {
        assert!(clock_reset(100.0, 99.0));
        assert!(clock_reset(100.0, 0.0));
        assert!(clock_reset(100.0, 106.0));
        assert!(clock_reset(100.0, f64::NAN));
        assert!(!clock_reset(100.0, 100.0));
        assert!(!clock_reset(100.0, 100.5));
        assert!(!clock_reset(100.0, 104.9));
    }



    #[test]
    fn opposing_dodge_direction_respects_its_own_blend_weight() {
        // A dodge can follow or oppose the stock course; its own weight determines the blend.
        let p = on();
        let b = bounds();
        let ship = (0.0, -1000.0);
        let stock = (0.0, 200.0);
        let same = compose(ship, stock, (0.0, 1.0), 1.0, &b, &p).unwrap();
        assert!((same.1 - ship.1 - 200.0).abs() < 1e-3, "same side: stock unchanged");
        // opposite side at w = 1 cancels the stock: falls back to stock (None), the reversal is invisible
        assert!(compose(ship, stock, (0.0, -1.0), 1.0, &b, &p).is_none());
        // At weight 2 the dodge wins and reverses the stock course.
        let flipped = compose(ship, stock, (0.0, -1.0), 2.0, &b, &p).unwrap();
        assert!(flipped.1 < ship.1 - 100.0);
        // slightly radial stock + w = 1: the opposite side still comes out tangential only partly
        let mixed = compose(ship, (60.0, 190.0), (0.0, -1.0), 1.0, &b, &p).unwrap();
        assert!((mixed.0 - ship.0) > 150.0, "mostly radial: {mixed:?}");
    }

    // ---- deny the dive ----
    fn dive_params() -> Params {
        Params {
            enabled: true,
            dive_under_dwell_s: 1.0,
            dive_under_chance: 0.8,
            dive_level_weight: 0.0,
            ..Params::default()
        }
    }
    fn dive_input(now: f64, target: (f32, f32)) -> StepInput {
        StepInput { now, aim: None, ship: (0.0, -1000.0), wanted: (target.0, target.1), target }
    }

    #[test]
    fn cone_is_straight_below_or_above_within_range() {
        let p = dive_params();
        assert!(in_dive_cone((0.0, 300.0), &p, 1.0)); // straight below
        assert!(in_dive_cone((0.0, -300.0), &p, 1.0)); // straight above
        assert!(in_dive_cone((200.0, 300.0), &p, 1.0)); // 34 deg off vertical
        assert!(!in_dive_cone((320.0, 300.0), &p, 1.0)); // 47 deg off vertical
        assert!(!in_dive_cone((400.0, 100.0), &p, 1.0)); // level
        assert!(!in_dive_cone((0.0, 700.0), &p, 1.0)); // out of range
        assert!(!in_dive_cone((0.0, 0.0), &p, 1.0));
        let under_only = Params { dive_over: false, ..p };
        assert!(in_dive_cone((0.0, 300.0), &under_only, 1.0) && !in_dive_cone((0.0, -300.0), &under_only, 1.0));
        assert!(!in_dive_cone((f32::NAN, 300.0), &p, 1.0));
    }

    #[test]
    fn dive_waits_for_dwell_then_sidesteps_then_cools_down() {
        let p = Params { dive_chance: 1.0, dive_under_chance: 1.0, ..dive_params() };
        let b = bounds();
        let mut rng = Rng::new(7);
        let mut st = ShipState::new(0.0);
        let tgt = (0.0, -700.0); // 300 u straight below the ship (ship y -1000, y points down)
        assert_eq!(step(&p, &mut st, &b, &dive_input(0.0, tgt), &mut rng).state, State::Stock);
        assert_eq!(step(&p, &mut st, &b, &dive_input(0.5, tgt), &mut rng).state, State::Stock);
        let r = step(&p, &mut st, &b, &dive_input(1.0, tgt), &mut rng);
        assert_eq!(r.state, State::Dive);
        let (wx, wy) = r.waypoint.expect("waypoint");
        assert!((wx.abs() > 150.0) && (wy + 1000.0).abs() < 100.0, "mostly sideways: {wx} {wy}");
        // keeps going while the dodge lasts, sticking to its side
        let side = wx.signum();
        let r2 = step(&p, &mut st, &b, &dive_input(1.5, tgt), &mut rng);
        assert_eq!(r2.state, State::Dive);
        assert_eq!(r2.waypoint.unwrap().0.signum(), side);
        // after the dodge (max 3 s) + cooldown (2 s) the dwell starts again
        assert_eq!(step(&p, &mut st, &b, &dive_input(4.1, tgt), &mut rng).state, State::Stock);
        assert_eq!(step(&p, &mut st, &b, &dive_input(5.0, tgt), &mut rng).state, State::Stock);
        assert_eq!(step(&p, &mut st, &b, &dive_input(6.2, tgt), &mut rng).state, State::Stock); // dwell restarts
        assert_eq!(step(&p, &mut st, &b, &dive_input(7.3, tgt), &mut rng).state, State::Dive);
    }

    #[test]
    fn leaving_the_cone_resets_the_dwell() {
        let p = Params { dive_chance: 1.0, dive_under_chance: 1.0, ..dive_params() };
        let b = bounds();
        let mut rng = Rng::new(3);
        let mut st = ShipState::new(0.0);
        step(&p, &mut st, &b, &dive_input(0.0, (0.0, -700.0)), &mut rng);
        step(&p, &mut st, &b, &dive_input(0.8, (500.0, -1000.0)), &mut rng); // level: out
        assert_eq!(step(&p, &mut st, &b, &dive_input(1.2, (0.0, -700.0)), &mut rng).state, State::Stock);
        assert_eq!(step(&p, &mut st, &b, &dive_input(2.3, (0.0, -700.0)), &mut rng).state, State::Dive);
    }

    #[test]
    fn roll_is_once_per_alignment_and_hits_about_the_chance() {
        let p = Params { dive_chance: 0.8, dive_under_chance: 0.8, ..dive_params() };
        let b = bounds();
        let mut rng = Rng::new(99);
        let (mut dives, mut skips) = (0u32, 0u32);
        for _ in 0..2000 {
            let mut st = ShipState::new(0.0);
            let tgt = (0.0, -700.0);
            step(&p, &mut st, &b, &dive_input(0.0, tgt), &mut rng);
            let first = step(&p, &mut st, &b, &dive_input(1.0, tgt), &mut rng).state;
            match first {
                State::Dive => dives += 1,
                State::DiveSkip => {
                    skips += 1;
                    // a declined alignment stays declined: no re-roll while the target stays in the cone
                    for k in 0..20 {
                        let s = step(&p, &mut st, &b, &dive_input(1.1 + 0.1 * k as f64, tgt), &mut rng).state;
                        assert_eq!(s, State::DiveSkip);
                    }
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        let rate = dives as f32 / (dives + skips) as f32;
        assert!((rate - 0.8).abs() < 0.04, "rate {rate}");
    }

    #[test]
    fn dive_remains_available_without_acceleration_gate_and_respects_the_wall() {
        let p = Params { dive_chance: 1.0, dive_under_chance: 1.0, ..dive_params() };
        let b = bounds();
        let mut rng = Rng::new(5);
        let mut st = ShipState::new(0.0);
        let input = |now: f64| dive_input(now, (0.0, -700.0));
        step(&p, &mut st, &b, &input(0.0), &mut rng);
        let dodge = step(&p, &mut st, &b, &input(1.5), &mut rng);
        assert_eq!(dodge.state, State::Dive);
        assert!(dodge.waypoint.is_some());
        // near the right wall the sidestep goes left
        for seed in 0..20 {
            let mut r = Rng::new(seed);
            assert_eq!(dive_side(1400.0, &b, &p, &mut r, 0.0), -1.0);
            assert_eq!(dive_side(-1400.0, &b, &p, &mut r, 0.0), 1.0);
        }
    }

    #[test]
    fn wrong_foot_goes_against_the_targets_sideways_motion() {
        let p = dive_params();
        let b = bounds();
        let mut r = Rng::new(1);
        for _ in 0..20 {
            assert_eq!(dive_side(0.0, &b, &p, &mut r, 90.0), -1.0);
            assert_eq!(dive_side(0.0, &b, &p, &mut r, -90.0), 1.0);
        }
        // slow target: random (both sides appear)
        let sides: Vec<f32> = (0..40).map(|_| dive_side(0.0, &b, &p, &mut r, 5.0)).collect();
        assert!(sides.contains(&-1.0) && sides.contains(&1.0));
        // the wall rule wins over the wrong-foot
        assert_eq!(dive_side(1400.0, &b, &p, &mut r, -90.0), -1.0);
        // switched off
        let off = Params { dive_wrongfoot_speed: 0.0, ..p };
        let sides: Vec<f32> = (0..40).map(|_| dive_side(0.0, &b, &off, &mut r, 90.0)).collect();
        assert!(sides.contains(&1.0));
    }

    #[test]
    fn target_velocity_is_estimated_and_reset_after_a_gap() {
        let mut st = ShipState::new(0.0);
        for k in 0..10 {
            track_target(&mut st, 0.1 * k as f64, (80.0 * 0.1 * k as f32, 0.0));
        }
        assert!((st.target_vx - 80.0).abs() < 5.0, "{}", st.target_vx);
        track_target(&mut st, 5.0, (0.0, 0.0)); // long gap: restart
        assert_eq!(st.target_vx, 0.0);
    }

    #[test]
    fn under_has_its_own_dwell_chance_and_a_level_pull() {
        // defaults: under = 0.5 s dwell, 100%; over = 1.0 s, 80%
        let p = Params { enabled: true, ..Params::default() };
        let b = bounds();
        let mut rng = Rng::new(11);
        let mut st = ShipState::new(0.0);
        let below = (0.0, -700.0); // ship y -1000, target 300 u lower
        assert_eq!(step(&p, &mut st, &b, &dive_input(0.0, below), &mut rng).state, State::Stock);
        let r = step(&p, &mut st, &b, &dive_input(0.5, below), &mut rng);
        assert_eq!(r.state, State::Dive, "under dodge after 0.5 s");
        // the dodge pulls DOWN (+y) towards the target's altitude as well as sideways
        let (wx, wy) = r.waypoint.unwrap();
        assert!(wx.abs() > 100.0 && wy > -1000.0, "{wx} {wy}");
        // a target above: climbs (negative y) and needs the longer over dwell
        let mut st2 = ShipState::new(0.0);
        let above = (0.0, -1300.0);
        step(&p, &mut st2, &b, &dive_input(0.0, above), &mut rng);
        assert_eq!(step(&p, &mut st2, &b, &dive_input(0.5, above), &mut rng).state, State::Stock);
        let r2 = step(&p, &mut st2, &b, &dive_input(1.0, above), &mut rng);
        if r2.state == State::Dive {
            assert!(r2.waypoint.unwrap().1 < -1000.0);
        }
    }

    #[test]
    fn waypoint_is_clamped_into_the_arena() {
        let b = bounds(); // x +-1500, y min -2000
        let w = clamp_waypoint((1691.0, -1000.0), (1300.0, -1000.0), &b);
        assert_eq!(w, (1450.0, -1000.0));
        let w = clamp_waypoint((0.0, -2033.0), (0.0, -1900.0), &b);
        assert_eq!(w, (0.0, -1950.0));
        assert_eq!(clamp_waypoint((100.0, -500.0), (0.0, -400.0), &b), (100.0, -500.0));
        // ship already outside: pulled back in, still a real course
        let w = clamp_waypoint((1650.0, -1000.0), (1600.0, -1000.0), &b);
        assert_eq!(w, (1450.0, -1000.0));
        // clamping onto the ship itself would give no course: unchanged
        assert_eq!(clamp_waypoint((1600.0, -1000.0), (1450.0, -1000.0), &b), (1600.0, -1000.0));
        assert_eq!(clamp_waypoint((f32::NAN, 0.0), (0.0, 0.0), &b).1, 0.0);
    }

    // ---- aim awareness ----
    fn aim_params() -> Params {
        Params { enabled: true, dive_enabled: false, aim_enabled: true, ..Params::default() }
    }

    #[test]
    fn aim_line_hits_ships_ahead_within_tolerance_and_slips_to_their_side() {
        let p = aim_params();
        let mut rng = Rng::new(4);
        let aim = AimInput { player: (0.0, 0.0), dir: (1.0, 0.0) }; // aiming along +x
        // 1000 ahead, 100 off to +y: on the line (tolerance 200); slips towards +y (the side it is on)
        let d = aim_dodge_dir(&aim, (1000.0, 100.0), &p, &mut rng).unwrap();
        assert!(d.0.abs() < 1e-5 && d.1 > 0.99, "{d:?}");
        let d = aim_dodge_dir(&aim, (1000.0, -100.0), &p, &mut rng).unwrap();
        assert!(d.1 < -0.99, "{d:?}");
        assert!(aim_dodge_dir(&aim, (1000.0, 300.0), &p, &mut rng).is_none()); // off the line
        assert!(aim_dodge_dir(&aim, (-1000.0, 0.0), &p, &mut rng).is_none()); // behind the player
        assert!(aim_dodge_dir(&aim, (2500.0, 0.0), &p, &mut rng).is_none()); // beyond aim_range
        // a diagonal aim
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let aim2 = AimInput { player: (0.0, 0.0), dir: (s, s) };
        let d = aim_dodge_dir(&aim2, (800.0, 900.0), &p, &mut rng).unwrap(); // slightly on the +y side
        assert!((d.0 * s + d.1 * s).abs() < 1e-4, "perpendicular to the aim: {d:?}");
        assert!(d.1 > 0.0);
    }

    #[test]
    fn the_awareness_window_opens_at_random_gaps_and_is_shared() {
        let p = aim_params();
        let mut rng = Rng::new(9);
        let mut clock = AimClock::default();
        let mut open_time = 0.0f64;
        let mut opens = 0;
        let mut was_open = false;
        let mut t = 100.0;
        while t < 160.0 {
            let open = clock.update(t, &p, &mut rng);
            if open {
                open_time += 0.05;
            }
            if open && !was_open {
                opens += 1;
            }
            was_open = open;
            t += 0.05;
        }
        // 60 s, a window of 1.5 s starting every 2.5..3.5 s (start to start): about 20 windows, open about half the time
        assert!((16..=24).contains(&opens), "{opens}");
        assert!((open_time / 60.0 - 0.5).abs() < 0.1, "{}", open_time / 60.0);
        // a clock restart re-arms
        assert!(!clock.update(5.0, &p, &mut rng));
    }

    #[test]
    fn step_slips_off_the_aim_line_before_the_dive_and_only_when_enabled() {
        let p = aim_params();
        let b = bounds();
        let mut rng = Rng::new(3);
        let mut st = ShipState::new(0.0);
        let aim = Some(AimInput { player: (-1000.0, -1000.0), dir: (1.0, 0.0) });
        let inp = StepInput { now: 1.0, aim, ship: (0.0, -950.0), wanted: (200.0, -950.0), target: (-1000.0, -1000.0) };
        let r = step(&p, &mut st, &b, &inp, &mut rng);
        assert_eq!(r.state, State::Aim);
        let (wx, wy) = r.waypoint.unwrap();
        assert!(wy > -950.0 + 100.0, "slips to the +y side (down): {wx} {wy}");
        let off = Params { aim_enabled: false, ..p };
        assert_eq!(step(&off, &mut st, &b, &inp, &mut rng).state, State::Stock);
    }

    #[test]
    fn early_fire_range_follows_reach() {
        let p = aim_params();
        assert!(early_fire_in_range(1200.0, Some(800.0), &p)); // 1.6 x 800 = 1280
        assert!(!early_fire_in_range(1300.0, Some(800.0), &p));
        assert!(!early_fire_in_range(100.0, None, &p));
        assert!(!early_fire_in_range(f32::NAN, Some(800.0), &p));
        // the window counter advances once per window
        let mut rng = Rng::new(5);
        let mut clock = AimClock::default();
        let mut t = 0.0;
        while t < 30.0 {
            clock.update(t, &p, &mut rng);
            t += 0.05;
        }
        assert!(clock.window >= 7 && clock.window <= 12, "{}", clock.window);
    }

    #[test]
    fn low_ships_never_steer_down() {
        let p = Params { enabled: true, ..Params::default() };
        let b = bounds();
        // ship at y=200 (low), stock heads down at the player; a tactic with a downward share must not go down
        for tactic in [(1.0f32, 0.9f32), (-1.0, 0.9), (0.0, 1.0)] {
            let w = compose((0.0, 200.0), (0.0, 400.0), tactic, 8.0, &b, &p);
            if let Some(w) = w {
                assert!(w.1 <= 200.0 + 1e-3, "{tactic:?} -> {w:?}");
            }
        }
        // high ships are unaffected
        let w = compose((0.0, -1000.0), (0.0, -800.0), (1.0, 0.5), 8.0, &b, &p).unwrap();
        assert!(w.1 > -1000.0);
    }
}
