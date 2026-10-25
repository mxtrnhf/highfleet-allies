//! The pure part of the "flare curtain": when the player's side launches a missile, an AI ship may start dropping
//! flares at random intervals BEFORE any missile is locked on to it, like a helicopter flooding the area with fake
//! heat sources. No game memory, no dependencies, host-testable:
//! `rustc --edition 2021 --test src/curtain_math.rs -o /tmp/cm_test && /tmp/cm_test`.

/// A launch older than this when a ship first looks at it is ignored (the ship was not around / not scanning).
pub const FRESH_LAUNCH_S: f64 = 1.0;

#[derive(Debug, Clone, Copy)]
pub struct Params {
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

impl Default for Params {
    fn default() -> Self {
        Self { probability: 0.0, interval_min: 0.5, interval_max: 1.0, seconds: 5.0, max_drops: 6, cooldown: 6.0, delay_min: 0.0, delay_max: 0.0, eta_lead: 4.0, eta_max_wait: 8.0 }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Curtain {
    /// The last launch number this ship looked at.
    pub seen_seq: u64,
    /// The curtain runs until this game time (0 = none).
    pub until: f64,
    pub next_drop: f64,
    pub quiet_until: f64,
    pub drops: u32,
    /// +1 / -1: which side of the line to the missile the ship slips to while the curtain runs.
    pub side: f32,
    /// The engine boost of the retreat is valid until this game time (set by the steering glue each tick it retreats).
    pub boost_until: f64,
    /// This curtain drops flares (false: the ship has no launcher or its rack is low; it still retreats).
    pub drops_enabled: bool,
    /// Most flares ever seen loaded in this ship's racks (the rack's capacity).
    pub rack_max: f32,
    /// The ship never had a flare (no launcher): it may shelter in an ally's flares instead of running.
    pub no_flares: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    None,
    Started,
    Extended,
    /// Release one flare launcher now.
    Drop,
    /// The curtain ended (time out, drop cap or a real panic took over).
    Ended,
}

fn range(lo: f32, hi: f32, rand: &mut dyn FnMut() -> f32) -> f64 {
    f64::from(lo + (hi - lo).max(0.0) * rand())
}

/// One step for one ship. `launch_seq` / `launch_time`: number and game time of the newest launch from the player's
/// side (0 = none yet). `panicking`: the ship is in a real missile panic; the curtain stops and never touches the
/// panic's timers.
pub fn update(
    c: &mut Curtain,
    p: &Params,
    now: f64,
    launch_seq: u64,
    launch_time: f64,
    panicking: bool,
    can_drop: bool,
    eligible: bool,
    eta: Option<f32>,
    rand: &mut dyn FnMut() -> f32,
) -> Event {
    if p.probability <= 0.0 {
        return Event::None;
    }
    if panicking {
        c.seen_seq = launch_seq;
        if c.until > 0.0 {
            c.until = 0.0;
            c.quiet_until = now + f64::from(p.cooldown);
            return Event::Ended;
        }
        return Event::None;
    }
    let mut event = Event::None;
    if launch_seq != c.seen_seq {
        c.seen_seq = launch_seq;
        if eligible && now - launch_time <= FRESH_LAUNCH_S {
            if c.until > 0.0 {
                c.until = c.until.max(now + f64::from(p.seconds));
                event = Event::Extended;
            } else if now >= c.quiet_until && rand() < p.probability {
                let delay = match eta.filter(|e| e.is_finite() && *e > 0.0) {
                    Some(e) => f64::from((e - p.eta_lead).clamp(p.delay_min, p.eta_max_wait.max(p.delay_min))),
                    None => range(p.delay_min, p.delay_max, rand),
                };
                c.until = now + delay + f64::from(p.seconds);
                c.drops = 0;
                c.drops_enabled = can_drop;
                c.side = if rand() < 0.5 { -1.0 } else { 1.0 };
                c.next_drop = now + delay;
                event = Event::Started;
            }
        }
    }
    if c.until > 0.0 {
        if now >= c.until || (c.drops_enabled && c.drops >= p.max_drops) {
            c.until = 0.0;
            c.quiet_until = now + f64::from(p.cooldown);
            return Event::Ended;
        }
        if c.drops_enabled && now >= c.next_drop && event == Event::None {
            c.drops += 1;
            c.next_drop = now + range(p.interval_min, p.interval_max, rand);
            return Event::Drop;
        }
    }
    event
}

/// How far behind the dropping ally (seen from the missile) a sheltering ship aims, in game units.
pub const SHELTER_OFFSET: f32 = 250.0;

/// A ship without flares "passes through the curtain" of an ally that drops them ("I got you, brother"): it heads for the
/// point `SHELTER_OFFSET` behind the dropping ship as seen from the missile, so the flares are between the missile and
/// the sheltering ship. Blended with its own course by `strength`. `None` on unusable numbers or when the missile sits on
/// the dropper.
pub fn shelter_waypoint(
    waypoint: (f32, f32),
    ship: (f32, f32),
    dropper: (f32, f32),
    missile: (f32, f32),
    strength: f32,
) -> Option<(f32, f32)> {
    let norm = |x: f32, y: f32| {
        let l = (x * x + y * y).sqrt();
        (l.is_finite() && l > 1.0e-4).then(|| (x / l, y / l, l))
    };
    let (ux, uy, _) = norm(dropper.0 - missile.0, dropper.1 - missile.1)?;
    let goal = (dropper.0 + ux * SHELTER_OFFSET, dropper.1 + uy * SHELTER_OFFSET);
    let (gx, gy, _) = norm(goal.0 - ship.0, goal.1 - ship.1)?;
    let (cx, cy, clen) = norm(waypoint.0 - ship.0, waypoint.1 - ship.1).unwrap_or((0.0, 0.0, 200.0));
    let strength = strength.clamp(0.0, 1.0);
    let (dx, dy, _) = norm(cx + (gx - cx) * strength, cy + (gy - cy) * strength)?;
    let len = if clen > 1.0 { clen } else { 200.0 };
    let out = (ship.0 + dx * len, ship.1 + dy * len);
    (out.0.is_finite() && out.1.is_finite()).then_some(out)
}

/// Ground guard of the retreat: above this y (y grows downward) the retreat may not point further down.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> Params {
        Params { probability: 1.0, ..Params::default() }
    }
    fn rnd() -> impl FnMut() -> f32 {
        let mut x = 0.3f32;
        move || {
            x = (x * 7.31 + 0.137).fract();
            x
        }
    }

    #[test]
    fn off_does_nothing() {
        let mut c = Curtain::default();
        let mut r = rnd();
        assert_eq!(update(&mut c, &Params::default(), 1.0, 1, 1.0, false, true, true, None, &mut r), Event::None);
        assert_eq!(c.seen_seq, 0);
    }

    #[test]
    fn launch_starts_then_drops_in_the_interval_then_caps() {
        let p = Params { max_drops: 3, ..on() };
        let mut c = Curtain::default();
        let mut r = rnd();
        assert_eq!(update(&mut c, &p, 10.0, 1, 10.0, false, true, true, None, &mut r), Event::Started);
        let mut t = 10.0;
        let mut drops = Vec::new();
        while t < 20.0 {
            t += 0.05;
            match update(&mut c, &p, t, 1, 10.0, false, true, true, None, &mut r) {
                Event::Drop => drops.push(t),
                Event::Ended => break,
                _ => {}
            }
        }
        assert_eq!(drops.len(), 3, "{drops:?}");
        for w in drops.windows(2) {
            let gap = w[1] - w[0];
            assert!((0.45..=1.1).contains(&gap), "gap {gap}");
        }
        assert!(c.quiet_until > t);
    }

    #[test]
    fn chance_is_rolled_once_per_launch_and_respects_probability() {
        let p = Params { probability: 0.5, ..Params::default() };
        let (mut hits, n) = (0, 2000);
        let mut r = rnd();
        for i in 0..n {
            let mut c = Curtain::default();
            if update(&mut c, &p, 5.0, 1 + i as u64, 5.0, false, true, true, None, &mut r) == Event::Started {
                hits += 1;
            }
            // looking at the same launch again never re-rolls
            assert_ne!(update(&mut c, &p, 5.1, 1 + i as u64, 5.0, false, true, true, None, &mut r), Event::Started);
        }
        let rate = hits as f32 / n as f32;
        assert!((rate - 0.5).abs() < 0.06, "rate {rate}");
    }

    #[test]
    fn second_launch_extends_and_old_launch_is_ignored() {
        let p = on();
        let mut c = Curtain::default();
        let mut r = rnd();
        assert_eq!(update(&mut c, &p, 10.0, 1, 10.0, false, true, true, None, &mut r), Event::Started);
        let until = c.until;
        assert_eq!(update(&mut c, &p, 12.0, 2, 12.0, false, true, true, None, &mut r), Event::Extended);
        assert!(c.until > until);
        let mut c2 = Curtain::default();
        assert_eq!(update(&mut c2, &p, 30.0, 5, 10.0, false, true, true, None, &mut r), Event::None); // 20 s old
        assert_eq!(c2.seen_seq, 5);
    }

    #[test]
    fn real_panic_takes_over_and_cooldown_blocks() {
        let p = on();
        let mut c = Curtain::default();
        let mut r = rnd();
        update(&mut c, &p, 10.0, 1, 10.0, false, true, true, None, &mut r);
        assert_eq!(update(&mut c, &p, 10.5, 1, 10.0, true, true, true, None, &mut r), Event::Ended);
        assert_eq!(c.until, 0.0);
        // a launch right after: cooldown (6 s) blocks a new curtain
        assert_eq!(update(&mut c, &p, 11.0, 2, 11.0, false, true, true, None, &mut r), Event::None);
        assert_eq!(update(&mut c, &p, 17.0, 3, 17.0, false, true, true, None, &mut r), Event::Started);
    }

    #[test]
    fn away_moves_away_from_the_missile_and_never_down_when_low() {
        let ship = (0.0, -1000.0);
        let stock = (0.0, -800.0); // stock heads down (+y) towards a missile below
        let missile = (0.0, -700.0); // 300 u below the ship
        let w = away_waypoint(stock, ship, missile, 1.0, 1.0).unwrap();
        assert!(w.1 < ship.1, "should climb away from a missile below: {w:?}");
        // a missile above: retreat goes down, unless the ship is low
        let w = away_waypoint((0.0, -1000.0), ship, (0.0, -1300.0), 1.0, 1.0).unwrap();
        assert!(w.1 > ship.1);
        let low = (0.0, 100.0);
        let w = away_waypoint((0.0, 100.0), low, (0.0, -200.0), 1.0, 1.0).unwrap();
        assert!(w.1 <= low.1 + 1e-3, "no descent when low: {w:?}");
        // strength 0 = own course; missile on top of the ship = None
        let w = away_waypoint((200.0, -1000.0), ship, missile, 1.0, 0.0).unwrap();
        assert!((w.0 - 200.0).abs() < 1e-3);
        assert!(away_waypoint(stock, ship, ship, 1.0, 1.0).is_none());
        assert!(away_waypoint(stock, ship, (f32::NAN, 0.0), 1.0, 1.0).is_none());
    }

    #[test]
    fn without_flares_the_curtain_runs_but_never_drops() {
        let p = on();
        let mut c = Curtain::default();
        let mut r = rnd();
        assert_eq!(update(&mut c, &p, 10.0, 1, 10.0, false, false, true, None, &mut r), Event::Started);
        let mut t = 10.0;
        let mut ended = false;
        while t < 20.0 {
            t += 0.05;
            match update(&mut c, &p, t, 1, 10.0, false, false, true, None, &mut r) {
                Event::Drop => panic!("dropped without flares"),
                Event::Ended => {
                    ended = true;
                    break;
                }
                _ => assert!(c.until > 0.0),
            }
        }
        assert!(ended && (t - 15.0).abs() < 0.2, "ended at {t}");
    }

    #[test]
    fn an_ineligible_ship_ignores_the_launch() {
        let p = on();
        let mut c = Curtain::default();
        let mut r = rnd();
        assert_eq!(update(&mut c, &p, 10.0, 1, 10.0, false, true, false, None, &mut r), Event::None);
        assert_eq!(c.seen_seq, 1);
        assert_eq!(c.until, 0.0);
        // the next launch, with the ship now eligible, can start one
        assert_eq!(update(&mut c, &p, 12.0, 2, 12.0, false, true, true, None, &mut r), Event::Started);
    }

    #[test]
    fn shelter_heads_for_the_point_behind_the_dropper() {
        let ship = (0.0, 0.0);
        let dropper = (500.0, 0.0);
        let missile = (1500.0, 0.0); // the missile comes from +x; "behind" the dropper is towards -x of it, at x = 250
        let w = shelter_waypoint((0.0, 100.0), ship, dropper, missile, 1.0).unwrap();
        assert!(w.0 > 99.0 && w.1.abs() < 1.0, "{w:?}"); // heads along +x towards (250, 0)
        // strength 0 keeps the own course
        let w = shelter_waypoint((0.0, 200.0), ship, dropper, missile, 0.0).unwrap();
        assert!(w.0.abs() < 1e-3 && w.1 > 190.0);
        assert!(shelter_waypoint((0.0, 100.0), ship, missile, missile, 1.0).is_none());
        assert!(shelter_waypoint((0.0, 100.0), ship, (f32::NAN, 0.0), missile, 1.0).is_none());
    }

    #[test]
    fn the_first_drop_waits_for_the_delay() {
        let p = Params { delay_min: 1.0, delay_max: 2.0, ..on() };
        let mut c = Curtain::default();
        let mut r = rnd();
        assert_eq!(update(&mut c, &p, 10.0, 1, 10.0, false, true, true, None, &mut r), Event::Started);
        let mut first_drop = None;
        let mut t = 10.0;
        while t < 20.0 && first_drop.is_none() {
            t += 0.02;
            if update(&mut c, &p, t, 1, 10.0, false, true, true, None, &mut r) == Event::Drop {
                first_drop = Some(t - 10.0);
            }
        }
        let d = first_drop.expect("a drop");
        assert!((0.99..=2.05).contains(&d), "first drop after {d}");
        // the curtain still lasts `seconds` after the delay
        assert!(c.until - 10.0 >= p.seconds as f64 + 1.0 - 1e-6);
    }

    #[test]
    fn the_eta_sets_the_first_drop() {
        let p = Params { delay_min: 0.3, delay_max: 2.0, ..on() };
        for (eta, expect) in [(2.0f32, 0.3f64), (8.0, 4.0), (30.0, 8.0)] {
            let mut c = Curtain::default();
            let mut r = rnd();
            update(&mut c, &p, 10.0, 1, 10.0, false, true, true, Some(eta), &mut r);
            assert!((c.next_drop - 10.0 - expect).abs() < 1e-3, "eta {eta}: {}", c.next_drop - 10.0);
        }
    }
}
