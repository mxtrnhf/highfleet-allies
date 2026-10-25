//! Ally ammunition: the AI weapon controller re-sets the ammo of group-0 guns (calibre 100..203 mm) every tick from two
//! creature bytes, `+0xc7c` (AP) and `+0xc7d` (proximity). The enemy battle spawner rolls them once per ship; an ally
//! is built by our own deploy call, so both stay 0 and it always fires plain shells whatever the pre-combat screen says.
//! This gives each ally the same dice (`hf_notes/docs/findings/ai/ai-tick-patch-sites.md` section 3), once, when it has
//! guns and a rating. Nothing is written for 37/57 mm lead guns (the stock rule).

const OFF_C7C: usize = 0xc7c;
const OFF_C7D: usize = 0xc7d;
const OFF_RATING: usize = 0x15f8;
const OFF_GROUP0: (usize, usize) = (0x13a8, 0x13b0);
const PART_STATS: usize = 0x188;
const STAT_CAL: usize = 0x144;
const STAT_LIST_BASE: usize = 0x1_43a3_3bf0;
const STAT_LIST_END: usize = 0x1_43a3_3bf8;

unsafe fn rd<T: Copy>(a: usize) -> T {
    (a as *const T).read_unaligned()
}

/// Calibre key of the first gun of group 0, `None` while the list is empty or the record is unreadable.
unsafe fn first_cal(ship: usize) -> Option<i32> {
    let (b, e): (usize, usize) = (rd(ship + OFF_GROUP0.0), rd(ship + OFF_GROUP0.1));
    if b < 0x10000 || e < b + 0x18 || (e - b) / 0x18 > 64 {
        return None;
    }
    let part: usize = rd(b + 8);
    if !crate::plausible(part) {
        return None;
    }
    let idx: i32 = rd(part + PART_STATS);
    let (base, end): (usize, usize) = (rd(STAT_LIST_BASE), rd(STAT_LIST_END));
    if idx < 0 || base == 0 || end < base || idx as usize >= (end - base) / 8 {
        return None;
    }
    let record: usize = rd(base + idx as usize * 8);
    crate::plausible(record).then(|| rd::<i32>(record + STAT_CAL))
}

/// Uniform in 0..1 from a xorshift stream seeded by the clock (two rolls in a row must not correlate).
fn unit() -> f32 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0);
    let mut x = STATE.load(Ordering::Relaxed);
    if x == 0 {
        x = 0x9e37_79b9_7f4a_7c15 ^ (crate::pick(1 << 30) as u64 + 1);
    }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    STATE.store(x, Ordering::Relaxed);
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// Rolls once for `ship`; returns false when it is not ready yet (try again next tick).
pub unsafe fn roll(ship: usize) -> bool {
    let Some(cal) = first_cal(ship) else { return false };
    let rating: f32 = rd(ship + OFF_RATING);
    if rating <= 0.0 && cal < 25 {
        return false;
    }
    let p = if cal >= 25 { 1.0 } else { (rating / 50.0).clamp(0.0, 1.0) };
    let (mut ap, mut prox) = (0u8, 0u8);
    if cal > 10 && unit() < p {
        let to_prox = if cal >= 25 { 0.8 } else { 0.2 };
        if unit() <= to_prox {
            prox = 1;
        } else {
            ap = 1;
        }
    }
    (ship as *mut u8).byte_add(OFF_C7C).write(ap);
    (ship as *mut u8).byte_add(OFF_C7D).write(prox);
    log::info!("ALLY ammo doctrine: c={ship:#x} cal={cal} rating={rating:.1} p={p:.2} -> AP={ap} proximity={prox}");
    true
}
