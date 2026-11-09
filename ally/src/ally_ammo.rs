//! Ally ammunition: the AI weapon controller re-sets the ammo of group-0 guns (calibre 100..203 mm) every tick from two
//! creature bytes, `+0xc7c` (AP) and `+0xc7d` (proximity). The enemy battle spawner rolls them once per ship; an ally
//! is built by our own deploy call, so both stay 0 and it always fires plain shells whatever the pre-combat screen says.
//! This gives each ally the same dice (`hf_notes/docs/findings/ai/ai-tick-patch-sites.md` section 3), once, when it has
//! guns and a rating. Nothing is written for 37/57 mm lead guns (the stock rule).

use std::mem::offset_of;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

use patchy::{PatchSession, ReturnType, Trampoline};

use crate::native::{live_ship, plausible, read, LINK_STRIDE, SHIP_LIST_LIMIT};
use crate::native_layout::{Body, Creature, Link, PartStats};

const OFF_C7C: usize = 0xc7c;
const OFF_C7D: usize = 0xc7d;
const OFF_RATING: usize = 0x15f8;
const OFF_GROUP0: (usize, usize) = (
    offset_of!(Creature, gun_group_0),
    offset_of!(Creature, gun_group_0) + std::mem::size_of::<usize>(),
);
const PART_STATS: usize = offset_of!(Body, part_index);
const STAT_CAL: usize = offset_of!(PartStats, caliber);
const STAT_LIST_BASE: usize = 0x1_43a3_3bf0;
const STAT_LIST_END: usize = 0x1_43a3_3bf8;
/// Byte stride of one gun link in the group-0 list: the part pointer sits at +8.
const GUN_LINK_STRIDE: usize = std::mem::size_of::<Link>();
/// Ceiling on the gun links read from group 0; a group has far fewer, so a larger count is a corrupt header.
const GUN_LIST_LIMIT: usize = 64;

unsafe fn rd<T: Copy>(a: usize) -> T {
    (a as *const T).read_unaligned()
}

/// Calibre key of the first gun of group 0, `None` while the list is empty or the record is unreadable.
unsafe fn first_cal(ship: usize) -> Option<i32> {
    let (b, e): (usize, usize) = (rd(ship + OFF_GROUP0.0), rd(ship + OFF_GROUP0.1));
    if b < 0x10000 || e < b {
        return None;
    }
    let list_bytes = e - b;
    if list_bytes < GUN_LINK_STRIDE || list_bytes / GUN_LINK_STRIDE > GUN_LIST_LIMIT {
        return None;
    }
    let part: usize = rd(b + offset_of!(Link, target));
    if !plausible(part) {
        return None;
    }
    let idx: i32 = rd(part + PART_STATS);
    let (base, end): (usize, usize) = (rd(STAT_LIST_BASE), rd(STAT_LIST_END));
    if idx < 0 || base < 0x10000 || end < base {
        return None;
    }
    let stat_count = (end - base) / std::mem::size_of::<usize>();
    if idx as usize >= stat_count {
        return None;
    }
    let record: usize = rd(base + idx as usize * std::mem::size_of::<usize>());
    plausible(record).then(|| rd::<i32>(record + STAT_CAL))
}

/// Uniform in 0..1 from a xorshift stream seeded by the clock (two rolls in a row must not correlate).
fn unit() -> f32 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0);
    let mut x = STATE.load(Ordering::Relaxed);
    if x == 0 {
        x = 0x9e37_79b9_7f4a_7c15 ^ (crate::time::pick(1 << 30) as u64 + 1);
    }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    STATE.store(x, Ordering::Relaxed);
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// Rolls once for `ship`; returns false when it is not ready yet (try again next tick).
pub unsafe fn roll(ship: usize) -> bool {
    let Some(cal) = first_cal(ship) else {
        return false;
    };
    let rating: f32 = rd(ship + OFF_RATING);
    if rating <= 0.0 && cal < 25 {
        return false;
    }
    let p = if cal >= 25 {
        1.0
    } else {
        (rating / 50.0).clamp(0.0, 1.0)
    };
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

// --- battle ammo lifecycle ------------------------------------------------------------------

static AMMO_ON: AtomicBool = AtomicBool::new(false);
/// Allies that have had their ammunition doctrine rolled this battle.
static AMMO_ROLLED: Mutex<Vec<usize>> = Mutex::new(Vec::new());
/// Per-gun ammo index and extra index that the battle reserve was funded with, recorded when the ally is
/// deployed (before the AI starts re-setting `+0x850`). `(creature, gun, index, extra_index)`.
static AMMO_FUNDING: Mutex<Vec<(usize, usize, i32, i32)>> = Mutex::new(Vec::new());

/// ReturnCreatureGunAmmoToInventory (0x1402698c0, RCX = creature): the end-of-battle refund.
/// The game credits each gun's pool to its current ammo index. The free base-shell pool starts
/// at 9999 rounds (`InitBattleGunAmmoReserves`, 0x14025fa30); an AI doctrine switch must not
/// turn that into 9999 priced rounds. Funding records preserve the original paid/free identity.
const REFUND: usize = 0x1_4026_98c0;
const REFUND_STOLEN: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10];
const REFUND_GUN_LISTS: [usize; 2] = [
    offset_of!(Creature, gun_group_0),
    offset_of!(Creature, gun_group_1),
];

/// Installs the refund detour when `ally_ai_ammo` is enabled. A differing site is logged and
/// disables the doctrine roll, exactly as before.
/// Returns patch-construction errors without committing the caller's session.
///
/// # Safety
/// Call during serialized initialization in the verified Steam 1.163 preferred-base image.
/// The native refund entry must be mapped code and remain unchanged until session commit.
pub(crate) unsafe fn install_refund(
    session: &mut PatchSession,
    enabled: bool,
) -> Result<(), String> {
    if !enabled {
        return Ok(());
    }
    let found = std::slice::from_raw_parts(REFUND as *const u8, REFUND_STOLEN.len());
    if found == REFUND_STOLEN {
        let mut trampoline = Trampoline::new();
        trampoline.bytes(&[0x48, 0x83, 0xec, 0x08]);
        trampoline.preserved_call(on_refund as *const (), &[], &[], ReturnType::None);
        trampoline.bytes(&[0x48, 0x83, 0xc4, 0x08]);
        trampoline.bytes(&REFUND_STOLEN);
        trampoline.relative_jump(REFUND + REFUND_STOLEN.len());
        session
            .detour_trampoline(REFUND, REFUND_STOLEN.len(), trampoline)
            .map_err(|e| e.to_string())?;
    } else {
        log::warn!("hf_ally: refund function bytes differ ({found:02x?}), not hooked: allies on AP / proximity would credit free ammo");
        set_enabled(false);
    }
    Ok(())
}

pub(crate) fn enabled() -> bool {
    AMMO_ON.load(Ordering::Relaxed)
}

pub(crate) fn set_enabled(enabled: bool) {
    AMMO_ON.store(enabled, Ordering::Relaxed);
}

/// Clears both per-battle ammo tables in the same order as the old battle-start reset.
pub(crate) fn clear_battle() {
    if let Ok(mut v) = AMMO_ROLLED.lock() {
        v.clear();
    }
    if let Ok(mut v) = AMMO_FUNDING.lock() {
        v.clear();
    }
}

/// Rolls the doctrine for every live ally that has not been rolled yet this battle.
///
/// # Safety
/// `allies` is the battle's borrowed ally list, read on the game thread while `ALLIES` is held.
/// The same lock order (`ALLIES -> AMMO_ROLLED`) as the old inline loop is preserved.
pub(crate) unsafe fn roll_new(ships: &[usize], allies: &[usize]) {
    let Ok(mut rolled) = AMMO_ROLLED.lock() else {
        return;
    };
    for &a in allies.iter().filter(|ship| ships.contains(ship)) {
        if live_ship(a) && !rolled.contains(&a) && roll(a) {
            rolled.push(a);
        }
    }
}

/// Remembers the ammo indices each gun of a freshly deployed ally was funded with.
///
/// # Safety
/// `creature` must be the game-owned ship just returned by the native deploy call, on the game thread.
pub(crate) unsafe fn record_funding(creature: usize) -> usize {
    let Ok(mut funding) = AMMO_FUNDING.lock() else {
        return 0;
    };
    record_funding_locked(&mut funding, creature)
}

/// Preserves the old handoff check/record sequence: the first lock failure aborts, while a
/// failure to take the second (recording) lock is ignored, exactly as `record_ammo_funding` was.
///
/// # Safety
/// Call on the game thread while the creature is still the live ship whose guns are being recorded.
pub(crate) unsafe fn ensure_funding_recorded(creature: usize) -> Result<(), ()> {
    let recorded = match AMMO_FUNDING.lock() {
        Ok(funding) => funding.iter().any(|entry| entry.0 == creature),
        Err(_) => return Err(()),
    };
    if recorded {
        return Ok(());
    }
    if let Ok(mut funding) = AMMO_FUNDING.lock() {
        record_funding_locked(&mut funding, creature);
    }
    Ok(())
}

unsafe fn record_funding_locked(
    funding: &mut Vec<(usize, usize, i32, i32)>,
    creature: usize,
) -> usize {
    let mut n = 0;
    for list in REFUND_GUN_LISTS {
        let (b, e): (usize, usize) = (read(creature + list), read(creature + list + 8));
        if b < 0x10000 || e < b || (e - b) / LINK_STRIDE > SHIP_LIST_LIMIT {
            continue;
        }
        for a in (b..e).step_by(LINK_STRIDE) {
            let gun: usize = read(a + 8);
            if !plausible(gun) {
                continue;
            }
            let entry = (
                creature,
                gun,
                read::<i32>(gun + offset_of!(Body, weapon_ammo)),
                read::<i32>(gun + offset_of!(Body, weapon_ammo_extra)),
            );
            if let Some(slot) = funding.iter_mut().find(|r| r.1 == gun) {
                *slot = entry;
            } else {
                funding.push(entry);
            }
            n += 1;
        }
    }
    n
}

/// Entry hook of the refund: for a creature whose ammo doctrine we rolled, put every gun back on the
/// ammo index its reserve was funded with.
/// Fleet creatures persist across battles, so pre-combat refunds must remain untouched: the
/// native selector refunds on every ammo pick, before the next spawn clears our battle tables.
///
/// # Safety
/// Called by the verified native refund detour on the game thread with the live creature in RCX.
unsafe extern "C" fn on_refund(creature: usize) {
    let _hook = crate::diag::scope("on_refund");
    let ours = AMMO_ROLLED.lock().map_or(false, |v| v.contains(&creature));
    if !ours {
        return;
    }
    // while the pre-combat screen is up the game refunds on every ammo pick; a partial list is treated as
    // "screen still up" so an uncertain scan cannot wipe the share the player just chose
    if crate::precombat::has_cards_or_uncertain() {
        return;
    }
    let Ok(funding) = AMMO_FUNDING.lock() else {
        return;
    };
    let (mut restored, mut unrecorded) = (0, 0);
    for list in REFUND_GUN_LISTS {
        let (b, e): (usize, usize) = (read(creature + list), read(creature + list + 8));
        if b < 0x10000 || e < b || (e - b) / LINK_STRIDE > SHIP_LIST_LIMIT {
            continue;
        }
        for a in (b..e).step_by(LINK_STRIDE) {
            let gun: usize = read(a + 8);
            if !plausible(gun) {
                continue;
            }
            if let Some(&(_, _, index, extra)) =
                funding.iter().find(|r| r.0 == creature && r.1 == gun)
            {
                (gun as *mut i32)
                    .byte_add(offset_of!(Body, weapon_ammo))
                    .write_unaligned(index);
                (gun as *mut i32)
                    .byte_add(offset_of!(Body, weapon_ammo_extra))
                    .write_unaligned(extra);
                restored += 1;
            } else {
                // no funding index (should not happen for an ally we deployed): keep the old safeguard
                for off in [
                    offset_of!(Body, weapon_ammo_amount),
                    offset_of!(Body, weapon_ammo_extra_amount),
                    offset_of!(Body, weapon_reload_step_rounds),
                ] {
                    ((gun + off) as *mut i32).write_unaligned(0);
                }
                ((gun + offset_of!(Body, weapon_ammo_loaded)) as *mut f32).write_unaligned(0.0);
                unrecorded += 1;
            }
        }
    }
    log::info!(
        "ALLY refund: c={creature:#x} {restored} guns restored to their funded ammo index, {unrecorded} without a record emptied"
    );
}
