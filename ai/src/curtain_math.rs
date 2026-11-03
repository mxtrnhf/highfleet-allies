//! The pure part of the "flare curtain": when the other camp launches a missile, an AI ship may start dropping
//! flares at random intervals BEFORE any missile is locked on to it, like a helicopter flooding the area with fake
//! heat sources. No game memory, no dependencies, host-testable:
//! `rustc --edition 2021 --test src/curtain_math.rs -o /tmp/cm_test && /tmp/cm_test`.

/// A launch older than this when a ship first looks at it is ignored (the ship was not around / not scanning).
pub const FRESH_LAUNCH_S: f64 = 1.0;

#[derive(Debug, Clone, Copy)]
pub struct CurtainParams {
    /// Chance, rolled once per ship per launch, that the ship starts a curtain. 0 = feature off.
    pub probability: f32,
    pub interval_min: f32,
    pub interval_max: f32,
    /// A curtain lasts this long (a further launch while it runs extends it).
    pub seconds: f32,
    /// Most drops one curtain may make (the flare racks are finite; the real panic needs them).
    pub max_drops: u32,
    /// After a curtain ends, no new one for this long.
    pub cooldown: f32,
    /// Wait between the launch and the first drop, a random time in `delay_min..delay_max` seconds.
    pub delay_min: f32,
    pub delay_max: f32,
    /// Flares live a fixed 5 s; with a missile ETA the first drop is `eta - eta_lead` s after the launch (at least
    /// `delay_min`, at most `eta_max_wait`), so the cloud is in the air when the missile arrives.
    pub eta_lead: f32,
    pub eta_max_wait: f32,
}

impl Default for CurtainParams {
    fn default() -> Self {
        Self { probability: 0.0, interval_min: 0.5, interval_max: 1.0, seconds: 5.0, max_drops: 6, cooldown: 6.0, delay_min: 0.0, delay_max: 0.0, eta_lead: 4.0, eta_max_wait: 8.0 }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Curtain {
    /// The last launch number this ship looked at.
    pub seen_launch: u64,
    /// The curtain starts acting at this game time (after the pre-drop ETA delay).
    pub active_from: f64,
    /// The curtain runs until this game time (0 = none).
    pub until: f64,
    pub next_drop: f64,
    pub quiet_until: f64,
    pub releases: u32,
    /// +1 / -1: which side of the line to the missile the ship slips to while the curtain runs.
    pub side: f32,
    /// The engine boost of the retreat is valid until this game time (set by the steering glue each tick it retreats).
    pub boost_until: f64,
    /// This curtain may release flares (false: the ship has no launcher or its rack is low; it still retreats).
    pub may_release: bool,
    /// Most flares ever seen loaded in this ship's racks (the rack's capacity).
    pub rack_capacity: f32,
}

impl Curtain {
    /// True once the initial ETA delay has elapsed and the curtain window is still open.
    pub fn is_active(&self, now: f64) -> bool {
        self.until > now && now >= self.active_from
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    None,
    Started,
    Extended,
    /// Release all flare launchers now.
    ReleaseFlares,
    /// The curtain ended (time out, drop cap or a real panic took over).
    Ended,
}

fn range(lo: f32, hi: f32, rand: &mut dyn FnMut() -> f32) -> f64 {
    f64::from(lo + (hi - lo).max(0.0) * rand())
}

/// One curtain step reads.
#[derive(Debug, Clone, Copy)]
pub struct CurtainInput {
    pub now: f64,
    /// Number and game time of the newest launch from the other camp (0 = none yet).
    pub launch_number: u64,
    pub launch_time: f64,
    /// The ship is in a real missile panic: the curtain stops and never touches the panic's timers.
    pub panicking: bool,
    /// The racks can release flares (a launcher exists and the reserve is held).
    pub can_release: bool,
    /// This ship is the launch target or a neighbour of it.
    pub eligible: bool,
    /// Estimated seconds until the missile arrives; `None` when unknown.
    pub eta: Option<f32>,
}

/// One step for one ship.
pub fn update(
    c: &mut Curtain,
    p: &CurtainParams,
    input: &CurtainInput,
    rand: &mut dyn FnMut() -> f32,
) -> Event {
    let CurtainInput { now, launch_number, launch_time, panicking, can_release, eligible, eta } = *input;
    if panicking {
        c.seen_launch = launch_number;
        if c.until > 0.0 {
            c.active_from = 0.0;
            c.until = 0.0;
            c.quiet_until = now + f64::from(p.cooldown);
            return Event::Ended;
        }
        return Event::None;
    }
    let mut event = Event::None;
    if launch_number != c.seen_launch {
        c.seen_launch = launch_number;
        if eligible && now - launch_time <= FRESH_LAUNCH_S {
            if c.until > 0.0 {
                c.until = c.until.max(now + f64::from(p.seconds));
                c.releases = 0;
                c.may_release = can_release;
                event = Event::Extended;
            } else if p.probability > 0.0 && now >= c.quiet_until && rand() < p.probability {
                let delay = match eta.filter(|e| e.is_finite() && *e > 0.0) {
                    Some(e) => f64::from((e - p.eta_lead).clamp(p.delay_min, p.eta_max_wait.max(p.delay_min))),
                    None => range(p.delay_min, p.delay_max, rand),
                };
                c.active_from = now + delay;
                c.until = c.active_from + f64::from(p.seconds);
                c.releases = 0;
                c.may_release = can_release;
                c.side = if rand() < 0.5 { -1.0 } else { 1.0 };
                c.next_drop = c.active_from;
                event = Event::Started;
            }
        }
    }
    if c.until > 0.0 {
        c.may_release = can_release;
        if now >= c.until || (c.may_release && c.releases >= p.max_drops) {
            c.active_from = 0.0;
            c.until = 0.0;
            c.quiet_until = now + f64::from(p.cooldown);
            return Event::Ended;
        }
        if c.may_release && now >= c.next_drop && event == Event::None {
            c.releases += 1;
            c.next_drop = now + range(p.interval_min, p.interval_max, rand);
            return Event::ReleaseFlares;
        }
    }
    event
}

/// Ground guard of the retreat: below this y (y grows downward, so larger y is lower) the retreat may not point
/// further down.
pub const NO_DESCENT_BELOW_Y: f32 = -250.0;

/// While a curtain runs the ship moves AWAY from the launched missile (retreat plus a little sidestep, `side`), blended
/// with its own course by `strength` (1 = fully away). Never downwards when the ship is low. `None` = leave the
/// waypoint (no usable numbers, or the missile is on top of the ship).
pub fn away_waypoint(
    waypoint: (f32, f32),
    ship: (f32, f32),
    missile: (f32, f32),
    side: f32,
    strength: f32,
) -> Option<(f32, f32)> {
    let norm = |x: f32, y: f32| {
        let l = (x * x + y * y).sqrt();
        (l.is_finite() && l > 1.0e-4).then(|| (x / l, y / l, l))
    };
    let (cx, cy, clen) = norm(waypoint.0 - ship.0, waypoint.1 - ship.1).unwrap_or((0.0, 0.0, 200.0));
    let (mx, my, _) = norm(missile.0 - ship.0, missile.1 - ship.1)?;
    let (ax, ay, _) = norm(-mx - my * side * 0.35, -my + mx * side * 0.35)?;
    let strength = strength.clamp(0.0, 1.0);
    let (mut dx, mut dy, _) = norm(cx + (ax - cx) * strength, cy + (ay - cy) * strength)?;
    if ship.1 > NO_DESCENT_BELOW_Y && dy > 0.0 {
        dy = 0.0;
        (dx, _, _) = norm(dx, 0.0)?;
    }
    let len = if clen > 1.0 { clen } else { 200.0 };
    let out = (ship.0 + dx * len, ship.1 + dy * len);
    (out.0.is_finite() && out.1.is_finite()).then_some(out)
}
