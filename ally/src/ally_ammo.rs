//! Ally ammunition: the AI weapon controller re-sets the ammo of group-0 guns (calibre 100..203 mm) every tick from two
//! creature bytes, `+0xc7c` (AP) and `+0xc7d` (proximity). The enemy battle spawner rolls them once per ship; an ally
//! is built by our own deploy call, so both stay 0 and it always fires plain shells whatever the pre-combat screen says.
//! This gives each ally the same dice (`hf_notes/docs/findings/ai/ai-tick-patch-sites.md` section 3), once, when it has
//! guns and a rating. Nothing is written for 37/57 mm lead guns (the stock rule).

use std::mem::{offset_of, size_of};
use std::ops::Range;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

use crate::patchy::{PatchSession, ReturnType, Trampoline};

use crate::native::{live_ship, plausible, read, LINK_STRIDE, SHIP_LIST_LIMIT};
use crate::native_layout::{Body, Creature, Link, PartStats};

const OFF_AP_DOCTRINE: usize = 0xc7c;
const OFF_PROXIMITY_DOCTRINE: usize = 0xc7d;
const OFF_RATING: usize = 0x15f8;
const OFF_GUN_GROUP_0: usize = offset_of!(Creature, gun_group_0);
const PART_STATS: usize = offset_of!(Body, part_index);
const STAT_CAL: usize = offset_of!(PartStats, caliber);
const STAT_LIST_BASE: usize = 0x1_43a3_3bf0;
const STAT_LIST_END: usize = 0x1_43a3_3bf8;

/// Bounds a gun group's vector header without dereferencing its entries.
///
/// # Safety
/// `ship` must be the live creature on the game thread; `group_offset` must locate its gun vector.
/// The returned range is only a header check, not proof of readable memory or object identity.
unsafe fn gun_link_range(ship: usize, group_offset: usize) -> Option<Range<usize>> {
    let begin: usize = read(ship + group_offset);
    let end: usize = read(ship + group_offset + size_of::<usize>());
    if begin < 0x10000 || end < begin {
        return None;
    }
    if (end - begin) / LINK_STRIDE > SHIP_LIST_LIMIT {
        return None;
    }

    Some(begin..end)
}

/// Calibre key of the first gun of group 0; `None` before a full link and plausible stats entry exist.
unsafe fn first_caliber(ship: usize) -> Option<i32> {
    let links = gun_link_range(ship, OFF_GUN_GROUP_0)?;
    if links.len() < LINK_STRIDE {
        return None;
    }

    let part: usize = read(links.start + offset_of!(Link, target));
    if !plausible(part) {
        return None;
    }

    let part_index: i32 = read(part + PART_STATS);
    let (base, end): (usize, usize) = (read(STAT_LIST_BASE), read(STAT_LIST_END));
    if part_index < 0 || base < 0x10000 || end < base {
        return None;
    }
    let stat_count = (end - base) / size_of::<usize>();
    if part_index as usize >= stat_count {
        return None;
    }

    let record: usize = read(base + part_index as usize * size_of::<usize>());
    plausible(record).then(|| read::<i32>(record + STAT_CAL))
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
///
/// # Safety
/// The caller must have established that this is a live ally on the game thread, with its
/// gun vectors and stats table valid for the duration of the roll.
unsafe fn roll(ship: usize) -> bool {
    const HEAVY_GUN_CALIBER: i32 = 25;
    const MIN_SPECIAL_AMMO_CALIBER: i32 = 10;

    let Some(caliber) = first_caliber(ship) else {
        return false;
    };
    let rating: f32 = read(ship + OFF_RATING);
    let heavy_gun = caliber >= HEAVY_GUN_CALIBER;
    if rating <= 0.0 && !heavy_gun {
        return false;
    }
    let special_ammo_chance = if heavy_gun {
        1.0
    } else {
        (rating / 50.0).clamp(0.0, 1.0)
    };

    let (mut ap, mut prox) = (0u8, 0u8);
    if caliber > MIN_SPECIAL_AMMO_CALIBER && unit() < special_ammo_chance {
        let proximity_chance = if heavy_gun { 0.8 } else { 0.2 };
        if unit() <= proximity_chance {
            prox = 1;
        } else {
            ap = 1;
        }
    }

    (ship as *mut u8).byte_add(OFF_AP_DOCTRINE).write(ap);
    (ship as *mut u8)
        .byte_add(OFF_PROXIMITY_DOCTRINE)
        .write(prox);
    log::info!("ALLY ammo doctrine: c={ship:#x} cal={caliber} rating={rating:.1} p={special_ammo_chance:.2} -> AP={ap} proximity={prox}");
    true
}

// --- battle ammo lifecycle ------------------------------------------------------------------

static AMMO_ON: AtomicBool = AtomicBool::new(false);
/// Allies that have had their ammunition doctrine rolled this battle.
static AMMO_ROLLED: Mutex<Vec<usize>> = Mutex::new(Vec::new());
/// Per-gun ammo indices that the battle reserve was funded with, recorded before the AI resets `+0x850`.
static AMMO_FUNDING: Mutex<Vec<FundingRecord>> = Mutex::new(Vec::new());

struct FundingRecord {
    creature: usize,
    gun: usize,
    ammo_index: i32,
    extra_ammo_index: i32,
}

/// ReturnCreatureGunAmmoToInventory (0x1402698c0, RCX = creature): the end-of-battle refund.
/// The game credits each gun's pool to its current ammo index. The free base-shell pool starts
/// at 9999 rounds (`InitBattleGunAmmoReserves`, 0x14025fa30); an AI doctrine switch must not
/// turn that into 9999 priced rounds. Funding records preserve the original paid/free identity.
const REFUND: usize = 0x1_4026_98c0;
const REFUND_STOLEN: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10];
const REFUND_GUN_LISTS: [usize; 2] = [OFF_GUN_GROUP_0, offset_of!(Creature, gun_group_1)];

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
    if found != REFUND_STOLEN {
        log::warn!("hf_ally: refund function bytes differ ({found:02x?}), not hooked: allies on AP / proximity would credit free ammo");
        set_enabled(false);
        return Ok(());
    }

    let mut trampoline = Trampoline::new();
    trampoline.bytes(&[0x48, 0x83, 0xec, 0x08]);
    trampoline.preserved_call(on_refund as *const (), &[], &[], ReturnType::None);
    trampoline.bytes(&[0x48, 0x83, 0xc4, 0x08]);
    trampoline.bytes(&REFUND_STOLEN);
    trampoline.relative_jump(REFUND + REFUND_STOLEN.len());
    session
        .detour_trampoline(REFUND, REFUND_STOLEN.len(), trampoline)
        .map_err(|error| error.to_string())?;
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
    for &ally in allies {
        if !ships.contains(&ally) || !live_ship(ally) || rolled.contains(&ally) {
            continue;
        }
        if roll(ally) {
            rolled.push(ally);
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
        Ok(funding) => funding.iter().any(|entry| entry.creature == creature),
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

unsafe fn record_funding_locked(funding: &mut Vec<FundingRecord>, creature: usize) -> usize {
    let mut recorded = 0;
    for group_offset in REFUND_GUN_LISTS {
        let Some(links) = gun_link_range(creature, group_offset) else {
            continue;
        };
        for link in links.step_by(LINK_STRIDE) {
            let gun: usize = read(link + offset_of!(Link, target));
            if !plausible(gun) {
                continue;
            }
            let entry = FundingRecord {
                creature,
                gun,
                ammo_index: read(gun + offset_of!(Body, weapon_ammo)),
                extra_ammo_index: read(gun + offset_of!(Body, weapon_ammo_extra)),
            };
            if let Some(slot) = funding.iter_mut().find(|record| record.gun == gun) {
                *slot = entry;
            } else {
                funding.push(entry);
            }
            recorded += 1;
        }
    }
    recorded
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
    let ours = AMMO_ROLLED
        .lock()
        .is_ok_and(|rolled| rolled.contains(&creature));
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
    for group_offset in REFUND_GUN_LISTS {
        let Some(links) = gun_link_range(creature, group_offset) else {
            continue;
        };
        for link in links.step_by(LINK_STRIDE) {
            let gun: usize = read(link + offset_of!(Link, target));
            if !plausible(gun) {
                continue;
            }
            if let Some(record) = funding
                .iter()
                .find(|record| record.creature == creature && record.gun == gun)
            {
                (gun as *mut i32)
                    .byte_add(offset_of!(Body, weapon_ammo))
                    .write_unaligned(record.ammo_index);
                (gun as *mut i32)
                    .byte_add(offset_of!(Body, weapon_ammo_extra))
                    .write_unaligned(record.extra_ammo_index);
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
