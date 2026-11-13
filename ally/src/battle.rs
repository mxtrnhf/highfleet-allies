//! Battle lifecycle, ally state, deployment, retreat, summaries and prize handling.
//!
//! All state here is game-thread state. `ALLIES` holds borrowed game-owned creature pointers;
//! the read-only [`Allies`] guard never creates an owning value and never extends a lifetime.
//! Mutex poisoning and try-lock behaviour follow the callers in `ship_switch`, `honor_share`,
//! `bomber`, the panels and the reticles: blocking readers use [`allies`], re-entrant hooks use
//! [`try_allies`] / [`tracked`].
//!
//! Native contracts and deployment evidence: `hf_notes/docs/findings/battle-ally.md`.

use std::{
    mem::offset_of,
    ops::Deref,
    sync::{
        atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering},
        Mutex, MutexGuard,
    },
};

use patchy::{PatchSession, ProcessModule, ReturnType, Trampoline};

use crate::ally_ammo;
use crate::ally_panels;
use crate::battle_diagnostics;
use crate::bomber;
use crate::diag;
use crate::honor_share;
use crate::lifecycle_log;
use crate::native::{
    count, live_ship, plausible, read, roster, GetProfileFn, SetControlledFn, SetLinkFn,
    BATTLE_RESULT, CLOCK, CONTROLLED_LINKS, CREATURES_BEGIN, CREATURES_END, CREATURE_LIST_LIMIT,
    ENEMIES_BEGIN, ENEMIES_END, GET_PROFILE, GET_PROFILE_PROLOGUE, IMAGE_BASE, LINK_STRIDE,
    OFF_ACTIVE, OFF_AGGRESSION, OFF_AI_FLAG, OFF_CODE, OFF_MODE_D70, OFF_POS_X, OFF_POS_Y,
    OFF_PRIZE, OFF_SIDE, OFF_STATE, OFF_TARGET, OFF_TARGET_LINK, RESERVE, ROSTER_BEGIN,
    SCENE_SELECTOR, SET_CONTROLLED, SET_CONTROLLED_PROLOGUE, SET_LINK, SET_LINK_PROLOGUE,
};
use crate::native_layout::Creature;
use crate::plane_loadouts;
use crate::precombat;
use crate::ship_switch;
use crate::sound;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

/// The ship mode (`+0xd70`) an ally gets: 0 = the plain AI ship the enemies use.
/// The former `ally_mode` config was removed in v0.12.1: 1 is player-controlled and 2 is
/// strike/retreat, so only 0 is valid here (battle-ally.md, "ally_mode removed").
pub(crate) const ALLY_MODE: i32 = 0;
/// The player's deploy call site (`0x1402d6f7a` + 5) and the scenario state in which a battle starts.
const PLAYER_DEPLOY_CALLER_RVA: usize = 0x2d6f7f;
const BATTLE_START_SELECTOR: u32 = 1;
/// Native reserve deploy (Ghidra's `rammingCheckCollision__`), byte-checked before use.
const DEPLOY: usize = 0x1_402d_6e90;
const DEPLOY_PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18];
/// Returns a living ship to the fleet and removes it from the level; byte-checked before use.
const EXTRACT: usize = 0x1_4020_ef30;
const EXTRACT_PROLOGUE: [u8; 10] = [0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18];

/// Per-frame arcade battle manager `FUN_1402cc260` (called from the Combat scenario).
const BATTLE_RVA: usize = 0x2cc260;
/// `mov rax,rsp; push rdi; sub rsp,0xa0` (11 bytes; the next instruction reads RAX, which the hook preserves).
const BATTLE_PROLOGUE: [u8; 11] = [
    0x48, 0x8b, 0xc4, 0x57, 0x48, 0x81, 0xec, 0xa0, 0x00, 0x00, 0x00,
];

/// The AI target choice `FUN_140078b40(ship)` is called from the per-ship driver at two sites (`mov rcx,rbx; call`).
const TARGET_FN: usize = 0x1_4007_8b40;
const TARGET_SITES: [(usize, [u8; 5]); 2] = [
    (0x1_4007_3b93, [0xe8, 0xa8, 0x4f, 0x00, 0x00]),
    (0x1_4007_3c73, [0xe8, 0xc8, 0x4e, 0x00, 0x00]),
];
const ARENA_X_HIGH: usize = 0x1_403c_2bc0;
const ARENA_X_LOW: usize = 0x1_403c_2bc8;
const ARENA_Y_LOW: usize = 0x1_403c_2bcc;
const DEPLOY_K: usize = 0x1_4035_b974;
const DEPLOY_AGGR: usize = 0x1_4035_24d8;
const ALLY_SLOT: usize = 1;
const RETREAT_POINT: usize = 0x1_47fc_7638;

type DeployFn =
    unsafe extern "C" fn(usize, usize, *mut u64, u64, f32, f32, f32, f32, u32, i32) -> u8;
type ExtractFn = unsafe extern "C" fn(usize, u32);

static ALLY_DONE: AtomicBool = AtomicBool::new(false);
static ARMED: AtomicBool = AtomicBool::new(false);
static HANDOVER: AtomicBool = AtomicBool::new(false);
static RETREAT_POINT_SAVED: AtomicU64 = AtomicU64::new(0);
static RETREAT_KEY: AtomicU32 = AtomicU32::new(0);
static KEY_WAS_DOWN: AtomicBool = AtomicBool::new(false);
static ALLY_PTR: AtomicUsize = AtomicUsize::new(0);
static FLAGSHIP: AtomicUsize = AtomicUsize::new(0);
static FLAGSHIP_ALIVE: AtomicBool = AtomicBool::new(false);
static FLAGSHIP_STATE: AtomicI32 = AtomicI32::new(-1);
static PLAYER_SEEN: AtomicU64 = AtomicU64::new(0);
static ALLY_DELAY: AtomicU64 = AtomicU64::new(0);
static USED_MASK: AtomicU32 = AtomicU32::new(0);
static MAX_ALLIES: AtomicU32 = AtomicU32::new(1);
static ALLIES: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static LAST_DEPLOY: AtomicU64 = AtomicU64::new(0);
static BATTLE_LOGGED: AtomicBool = AtomicBool::new(false);
static SUM_SPAWNED: AtomicU32 = AtomicU32::new(0);
static SUM_LOST: AtomicU32 = AtomicU32::new(0);
static SUM_T0: AtomicU64 = AtomicU64::new(0);
static SUM_ENEMIES: AtomicU32 = AtomicU32::new(0);
static SUM_SQUAD: AtomicU32 = AtomicU32::new(0);
static SUM_ALLY_STATE: AtomicI32 = AtomicI32::new(-1);
static SUM_RESULT: AtomicI32 = AtomicI32::new(0);
static SUM_FLAGS: AtomicU32 = AtomicU32::new(0);
static SUM_LAST: AtomicU64 = AtomicU64::new(0);
static ANNOUNCED_LOST: Mutex<Vec<usize>> = Mutex::new(Vec::new());

const F_SPAWNED: u32 = 1;
const F_HANDOVER: u32 = 2;
const F_EXTRACTED: u32 = 4;
const F_OFF: u32 = 8;
const F_FLAGSHIP_LEFT: u32 = 16;
const F_RETREAT_KEY: u32 = 32;

/// Read-only view of the tracked ally pointers.
pub(crate) struct Allies(MutexGuard<'static, Vec<usize>>);

impl Deref for Allies {
    type Target = [usize];

    fn deref(&self) -> &Self::Target {
        self.0.as_slice()
    }
}

pub(crate) fn armed() -> bool {
    ARMED.load(Ordering::Relaxed)
}

pub(crate) fn allies() -> Option<Allies> {
    ALLIES.lock().ok().map(Allies)
}

pub(crate) fn try_allies() -> Option<Allies> {
    ALLIES.try_lock().ok().map(Allies)
}

/// Whether `ship` is an ally this mod deployed in the current battle.
pub(crate) fn tracked(ship: usize) -> bool {
    armed() && ALLIES.try_lock().is_ok_and(|allies| allies.contains(&ship))
}

/// Replaces `selected` with `previous` in `ALLIES` and ensures the original player's funded
/// ammo is recorded. `ALLIES` stays locked across the funding check, and the first
/// `AMMO_FUNDING` lock failure aborts exactly as before.
///
/// # Safety
/// Call on the game thread before native traversal, with `previous` and `selected` established
/// as current roster creatures. Their gun lists must remain valid throughout funding collection;
/// no native mutation or ownership transfer occurs while the Rust-owned ally list is locked.
pub(crate) unsafe fn prepare_handoff(previous: usize, selected: usize) -> bool {
    let Ok(mut allies) = ALLIES.lock() else {
        return false;
    };
    let Some(index) = allies.iter().position(|&ship| ship == selected) else {
        return false;
    };
    if ally_ammo::ensure_funding_recorded(previous).is_err() {
        return false;
    }
    allies[index] = previous;
    true
}

/// Exact failure-branch rollback: searches for `selected` again and writes it back if present.
pub(crate) fn rollback_handoff(selected: usize) {
    if let Ok(mut allies) = ALLIES.lock() {
        if let Some(index) = allies.iter().position(|&ship| ship == selected) {
            allies[index] = selected;
        }
    }
}

pub(crate) fn configure(config: &crate::config::Config) {
    MAX_ALLIES.store(config.max_allies.max(1), Ordering::Relaxed);
    FUEL_RETREAT_PERCENT.store(
        config.ally_fuel_retreat_percent.clamp(0.0, 100.0).to_bits(),
        Ordering::Relaxed,
    );
    let handover = match config.flagship_leaves.as_str() {
        "handover" => true,
        "retreat" => false,
        other => {
            log::warn!(
                "hf_ally: flagship_leaves {other:?} is not \"handover\" or \"retreat\"; using \"handover\""
            );
            true
        }
    };
    HANDOVER.store(handover, Ordering::Relaxed);
    RETREAT_KEY.store(config.retreat_key, Ordering::Relaxed);
    ALLY_DELAY.store(
        config.ally_delay_seconds.max(0.0).to_bits(),
        Ordering::Relaxed,
    );
}

/// Applies `spare_prizes` and installs the two AI target-choice detours when the native sites match.
/// Byte conflicts disable the feature with a warning; patch-construction failures return an error.
///
/// # Safety
/// Call during serialized initialization in the verified Steam 1.163 preferred-base image.
/// Native code must remain mapped and unmodified until the caller commits the patch session.
pub(crate) unsafe fn install_prize_guard(
    session: &mut PatchSession,
    enabled: bool,
) -> Result<(), String> {
    SPARE_PRIZES.store(enabled, Ordering::Relaxed);
    if !enabled {
        return Ok(());
    }
    let target_ok = std::slice::from_raw_parts(TARGET_FN as *const u8, 3) == [0x48, 0x8b, 0xc4];
    let sites_ok = TARGET_SITES
        .iter()
        .all(|(site, bytes)| std::slice::from_raw_parts(*site as *const u8, 5) == bytes);
    if target_ok && sites_ok {
        for (site, bytes) in TARGET_SITES {
            let mut trampoline = Trampoline::new();
            trampoline.bytes(&[0x51, 0x48, 0x83, 0xec, 0x28]); // push rcx; sub rsp, 0x28
            trampoline.absolute_call(TARGET_FN as *const ());
            trampoline.bytes(&[0x48, 0x83, 0xc4, 0x28, 0x59]); // add rsp, 0x28; pop rcx
            trampoline.preserved_call(on_target_chosen as *const (), &[], &[], ReturnType::None);
            trampoline.relative_jump(site + bytes.len());
            session
                .detour_trampoline(site, bytes.len(), trampoline)
                .map_err(|e| e.to_string())?;
        }
    } else {
        log::warn!(
            "hf_ally: the AI target choice or its call sites differ, prize ships are not spared"
        );
        SPARE_PRIZES.store(false, Ordering::Relaxed);
    }
    Ok(())
}

/// Verified addresses for the battle-manager entry point.
pub(crate) struct BattleHook {
    address: usize,
}

/// Checks the deploy and battle-manager sites before the ship-switch validation and the patch.
/// Returns an error on resolution failure or any required byte mismatch.
///
/// # Safety
/// Call during serialized initialization in the verified Steam 1.163 preferred-base image;
/// its fixed function addresses and the resolved entry point must be readable code.
pub(crate) unsafe fn verify_battle_hooks(module: &ProcessModule) -> Result<BattleHook, String> {
    for (name, address, expected) in [
        ("deploy", DEPLOY, &DEPLOY_PROLOGUE[..]),
        (
            "set_controlled",
            SET_CONTROLLED,
            &SET_CONTROLLED_PROLOGUE[..],
        ),
        ("set_link", SET_LINK, &SET_LINK_PROLOGUE[..]),
        ("get_profile", GET_PROFILE, &GET_PROFILE_PROLOGUE[..]),
        ("extract", EXTRACT, &EXTRACT_PROLOGUE[..]),
    ] {
        let found = std::slice::from_raw_parts(address as *const u8, expected.len());
        if found != expected {
            return Err(format!(
                "{name} at {address:#x} has unexpected bytes {found:02x?}"
            ));
        }
    }
    let address = module.resolve_rva(BATTLE_RVA).map_err(|e| e.to_string())?;
    let found = std::slice::from_raw_parts(address as *const u8, BATTLE_PROLOGUE.len());
    if found != BATTLE_PROLOGUE {
        return Err(format!(
            "battle manager at {address:#x} has unexpected bytes {found:02x?}"
        ));
    }
    Ok(BattleHook { address })
}

/// Installs the per-frame battle-manager entry hook at the verified address.
/// Returns patch-construction errors without committing the caller's session.
///
/// # Safety
/// `hook` must have been verified against the current image in this initialization pass,
/// with no intervening native code modifications at that site.
pub(crate) unsafe fn install_battle_hook(
    session: &mut PatchSession,
    hook: BattleHook,
) -> Result<(), String> {
    session
        .patch_call(
            hook.address,
            on_battle_tick as *const (),
            BATTLE_PROLOGUE.len(),
            true,
            ReturnType::None,
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Emits a pending previous-battle summary using its last observed game time, at most once.
///
/// # Safety
/// Call on the game thread while the supported image's battle-result global is readable.
pub(crate) unsafe fn emit_last_summary() {
    emit_summary(f64::from_bits(SUM_LAST.load(Ordering::Relaxed)));
}

static ALLY_LAST: Mutex<Vec<(usize, i32)>> = Mutex::new(Vec::new());

/// Announces allies that have just been destroyed; runs every battle frame. The game removes a destroyed ship from the
/// roster with its state still 1 (v0.10.1 test), so an ally that is gone counts as destroyed unless it was
/// retreating (state 2) or extracted, or no enemy is left (victory tears the roster down). A ship seen in state 6/7
/// still in the roster counts too.
unsafe fn ally_sound_events(ships: &[usize]) {
    let (Ok(mut announced), Ok(mut allies), Ok(mut last)) =
        (ANNOUNCED_LOST.lock(), ALLIES.lock(), ALLY_LAST.lock())
    else {
        return;
    };
    announced.retain(|p| ships.contains(p));
    for &p in allies.iter().filter(|p| ships.contains(p)) {
        let state = read::<i32>(p + OFF_STATE);
        match last.iter_mut().find(|e| e.0 == p) {
            Some(e) => e.1 = state,
            None => last.push((p, state)),
        }
    }
    let enemies = count(ENEMIES_BEGIN, ENEMIES_END, 0x18);
    let mut gone = Vec::new();
    for &p in allies.iter() {
        let here = ships.contains(&p);
        let last_state = last.iter().find(|e| e.0 == p).map_or(-1, |e| e.1);
        let dead = if here {
            matches!(read::<i32>(p + OFF_STATE), 6 | 7)
        } else {
            gone.push(p);
            let extracted = SUM_FLAGS.load(Ordering::Relaxed) & F_EXTRACTED != 0
                || matches!(last_state, 2 | 6 | 7 | -1);
            if extracted || enemies == 0 {
                log::info!("ALLY c={p:#x} left the roster, not destroyed (last state {last_state}, enemies {enemies})");
            }
            // retreated alive: flying out (state 2) when it vanished, or extracted with the flagship
            if SUM_FLAGS.load(Ordering::Relaxed) & F_EXTRACTED != 0 || last_state == 2 {
                sound::retreated();
            }
            !extracted && enemies != 0
        };
        if !dead || announced.contains(&p) {
            continue;
        }
        announced.push(p);
        SUM_LOST.fetch_add(1, Ordering::Relaxed);
        if ALLY_PTR.load(Ordering::Relaxed) == p {
            SUM_ALLY_STATE.store(6, Ordering::Relaxed);
        }
        log::info!("ALLY lost: c={p:#x} destroyed (last state {last_state}, in roster {here}, enemies {enemies})");
        diag::flush_log();
        sound::lost();
    }
    allies.retain(|p| !gone.contains(p));
    last.retain(|e| ships.contains(&e.0));
}

/// Native spawner callback; starts a new ally battle only at the player's initial deploy site.
///
/// # Safety
/// Call on the game thread with the native spawner's template (null or still-valid node) and
/// caller return address, before concurrent battle/native-list mutation can occur.
pub(crate) unsafe extern "C" fn on_spawn(template: usize, caller: usize) {
    let _hook = diag::scope("on_spawn");
    // a new battle begins when the player's ship is deployed at the start (selector 1); the ally's own
    // deploy goes through the same call site later (selector 2) and must not re-arm
    if caller.wrapping_sub(IMAGE_BASE) == PLAYER_DEPLOY_CALLER_RVA
        && read::<u32>(SCENE_SELECTOR) == BATTLE_START_SELECTOR
    {
        restore_orders("next battle", true);
        ALLY_DONE.store(false, Ordering::Relaxed);
        PLAYER_SEEN.store(0, Ordering::Relaxed);
        battle_diagnostics::reset();
        bomber::reset();
        ally_panels::invalidate();
        lifecycle_log::reset_spawn_budget();
        PRIZE_LINES.store(0, Ordering::Relaxed);
        let armed = precombat::take_seen();
        ARMED.store(armed, Ordering::Relaxed);
        log::info!(
            "ALLY battle start: scene selector {}, {}",
            read::<u32>(SCENE_SELECTOR),
            if armed {
                "armed (entered through the pre-combat screen)"
            } else {
                "NOT armed (no pre-combat screen: re-engaged chase or other battle), the mod stays out of it"
            }
        );
        diag::flush_log();
        emit_summary(f64::from_bits(SUM_LAST.load(Ordering::Relaxed)));
        ALLY_PTR.store(0, Ordering::Relaxed);
        FLAGSHIP.store(0, Ordering::Relaxed);
        RETREAT_POINT_SAVED.store(0, Ordering::Relaxed);
        if let Ok(mut v) = ALLIES.lock() {
            v.clear();
        }
        ally_ammo::clear_battle();
        USED_MASK.store(0, Ordering::Relaxed);
        LAST_DEPLOY.store(0, Ordering::Relaxed);
        BATTLE_LOGGED.store(false, Ordering::Relaxed);
        SUM_SPAWNED.store(0, Ordering::Relaxed);
        SUM_LOST.store(0, Ordering::Relaxed);
    }
    lifecycle_log::log_spawn(template, caller);
}

unsafe fn reserve_to_front(k: usize) -> bool {
    let sentinel: usize = read(RESERVE);
    if sentinel == 0 {
        return false;
    }
    let mut node: usize = read(sentinel);
    for _ in 0..k {
        if node == 0 || node == sentinel {
            return false;
        }
        node = read(node);
    }
    if node == 0 || node == sentinel {
        return false;
    }
    if k == 0 {
        return true;
    }
    let (next, prev): (usize, usize) = (read(node), read(node + 8));
    (prev as *mut usize).write_unaligned(next); // prev.next = next
    ((next + 8) as *mut usize).write_unaligned(prev); // next.prev = prev
    let first: usize = read(sentinel);
    (node as *mut usize).write_unaligned(first);
    ((node + 8) as *mut usize).write_unaligned(sentinel);
    ((first + 8) as *mut usize).write_unaligned(node);
    (sentinel as *mut usize).write_unaligned(node);
    true
}

/// Ships the F8 order has put on the bombing role (`+0xd70 = 2`): (ship, order before, checks that did not find it).
/// The role stays on the ship after it returns to the fleet (the game never resets it), so it is put back.
static ORDERED: Mutex<Vec<(usize, i32, u32)>> = Mutex::new(Vec::new());
pub(crate) const ORDER_BOMBING: i32 = 2;

/// Puts the order back on every ordered ship that is out of the battle and still in a player fleet. A ship that
/// is still fighting waits; a ship that is in no fleet (destroyed, object gone) is never touched.
/// `final_try`: the next battle starts, the list is cleared afterwards.
pub(crate) unsafe fn restore_orders(why: &str, final_try: bool) {
    let Ok(mut list) = ORDERED.lock() else { return };
    if list.is_empty() {
        return;
    }
    let in_battle = roster();
    list.retain_mut(|(ship, previous, misses)| {
        if in_battle.contains(ship) {
            return !final_try; // still fighting
        }
        if diag::fleet_has_ship(*ship) {
            let now = read::<i32>(*ship + OFF_MODE_D70);
            if now == ORDER_BOMBING {
                (*ship as *mut i32).byte_add(OFF_MODE_D70).write_unaligned(*previous);
                log::info!("ALLY order restored ({why}): c={:#x} order {now} -> {previous}", *ship);
            } else {
                log::info!("ALLY order not restored ({why}): c={:#x} order is {now} now, left alone", *ship);
            }
            return false;
        }
        *misses += 1;
        if final_try || *misses >= 40 {
            log::info!("ALLY order not restored ({why}): c={:#x} is in no player fleet (destroyed?), left alone", *ship);
            return false;
        }
        true
    });
}

/// Order the ally out: the AI-run, strike-craft "fly out" state (mode 2, state 2). The AI tick then steers to
/// the arena edge and the manager extracts the ship (it returns to the fleet if alive).
unsafe fn order_retreat(ally: usize, why: &str) -> bool {
    if !live_ship(ally) {
        log::info!("ALLY retreat ({why}): c={ally:#x} is not a live ship, ignored");
        return false;
    }
    let state = read::<i32>(ally + OFF_STATE);
    if matches!(state, 6 | 7) {
        log::info!("ALLY retreat ({why}): ally is dead (state {state}), nothing to do");
        return false;
    }
    if why == "key" {
        SUM_FLAGS.fetch_or(F_RETREAT_KEY, Ordering::Relaxed);
    }
    (ally as *mut u8).byte_add(OFF_AI_FLAG).write(1); // AI-run again (a promoted ally is player-flown)
    if let Ok(mut ordered) = ORDERED.lock() {
        if !ordered.iter().any(|e| e.0 == ally) {
            ordered.push((ally, read::<i32>(ally + OFF_MODE_D70), 0));
        }
    }
    (ally as *mut i32)
        .byte_add(OFF_MODE_D70)
        .write_unaligned(ORDER_BOMBING);
    (ally as *mut i32).byte_add(OFF_STATE).write_unaligned(2);
    log::info!(
        "ALLY retreat ordered ({why}): c={ally:#x} was state {state}, now mode_d70={} state={} ai_flag={}",
        read::<i32>(ally + OFF_MODE_D70),
        read::<i32>(ally + OFF_STATE),
        read::<u8>(ally + OFF_AI_FLAG),
    );
    diag::flush_log();
    true
}

static FUEL_RETREAT_PERCENT: AtomicU32 = AtomicU32::new(0); // f32 bits
static FUEL_LOG_AT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0); // f64 bits of the game clock
const OFF_FUEL: usize = offset_of!(Creature, fuel); // f64: combat fuel now (the battle burns it: 0x14007a210)
const OFF_FUEL_CAPACITY: usize = offset_of!(Creature, fuel_capacity); // f64
const OFF_FUEL_AT_START: usize = 0xbd0; // f32: written at spawn (0x1402672c0), not modeled by highfleet-rs

/// Orders out every ally whose combat fuel is at or below the configured percent. Once per ship: a ship already
/// ordered out (F8 or fuel) stays in `ORDERED` until it is back in the fleet.
unsafe fn fuel_retreat(allies: &[usize]) {
    let percent = f32::from_bits(FUEL_RETREAT_PERCENT.load(Ordering::Relaxed));
    if percent <= 0.0 {
        return;
    }
    for &ally in allies {
        if !live_ship(ally) || read::<i32>(ally + OFF_STATE) == 2 {
            continue;
        }
        // a promoted ally is the ship the player flies
        if read::<i32>(ally + OFF_MODE_D70) == 1 && read::<u8>(ally + OFF_AI_FLAG) == 0 {
            continue;
        }
        if ORDERED
            .lock()
            .map(|o| o.iter().any(|e| e.0 == ally))
            .unwrap_or(true)
        {
            continue;
        }
        let fuel: f64 = read(ally + OFF_FUEL);
        let basis: f64 = read(ally + OFF_FUEL_CAPACITY);
        if !fuel.is_finite() || !basis.is_finite() || basis <= 0.0 {
            continue;
        }
        let now = fuel / basis * 100.0;
        let clock = read::<f64>(CLOCK);
        if clock - f64::from_bits(FUEL_LOG_AT.load(Ordering::Relaxed)) >= 10.0 {
            FUEL_LOG_AT.store(clock.to_bits(), Ordering::Relaxed);
            log::info!(
                "ALLY fuel: c={ally:#x} combat fuel {fuel:.1}, capacity {:.1}, at battle start {:.1}, {now:.1}% of capacity",
                read::<f64>(ally + OFF_FUEL_CAPACITY),
                read::<f32>(ally + OFF_FUEL_AT_START)
            );
        }
        if now <= percent as f64 {
            log::info!(
                "ALLY fuel low: c={ally:#x} combat fuel {fuel:.1} of capacity {basis:.1} = {now:.1}% <= {percent}%, leaving"
            );
            order_retreat(ally, "fuel");
            sound::danger("fuel retreat");
        }
    }
}

/// Retreat key and automatic retreat; runs every battle frame.
unsafe fn retreat_logic(ships: &[usize]) {
    // an ally the manager promoted to the player's ship (mode 1, AI flag off) is the flagship now, not an ally
    if let Ok(mut v) = ALLIES.lock() {
        v.retain(|p| {
            let promoted = ships.contains(p)
                && read::<i32>(*p + OFF_MODE_D70) == 1
                && read::<u8>(*p + OFF_AI_FLAG) == 0;
            if promoted {
                log::info!(
                    "ALLY c={:#x} was promoted to the flagship: no longer tracked as an ally",
                    *p
                );
            }
            !promoted
        });
    }
    let allies: Vec<usize> = ALLIES
        .lock()
        .map(|v| v.iter().copied().filter(|p| ships.contains(p)).collect())
        .unwrap_or_default();
    fuel_retreat(&allies);
    let key = RETREAT_KEY.load(Ordering::Relaxed);
    if key != 0 {
        let down = GetAsyncKeyState(key as i32) as u16 & 0x8000 != 0;
        if down && !KEY_WAS_DOWN.swap(true, Ordering::Relaxed) {
            // v0.9.2: only while the player's own ship is flying in the arena. During the fade-out after a retreat
            // (flagship leaving or gone) the scene is being torn down and the order would hit a dying ship.
            let flagship_flying = ships.iter().any(|&p| {
                !allies.contains(&p)
                    && live_ship(p)
                    && read::<i32>(p + OFF_MODE_D70) == 1
                    && read::<i32>(p + OFF_STATE) != 2
            });
            if !flagship_flying {
                log::info!("ALLY retreat key ignored: no flagship is flying (battle ending or handover pending)");
            }
            let mut ordered = false;
            for &ally in allies.iter().filter(|_| flagship_flying) {
                // never on a ship the player flies (e.g. an ally after the flagship left): it would turn it into an
                // AI strike run; that ship retreats by flying to the retreat point
                if read::<i32>(ally + OFF_MODE_D70) == 1 && read::<u8>(ally + OFF_AI_FLAG) == 0 {
                    log::info!("ALLY retreat key ignored: c={ally:#x} is the ship you fly");
                } else {
                    ordered |= order_retreat(ally, "key");
                }
            }
            if ordered {
                sound::danger("retreat key");
            }
        } else if !down {
            KEY_WAS_DOWN.store(false, Ordering::Relaxed);
        }
    }
    let point: u64 = read(RETREAT_POINT);
    if point != 0 {
        RETREAT_POINT_SAVED.store(point, Ordering::Relaxed);
    }
    // the flagship: a flown (mode 1) ship that is not an ally
    let flagship = ships
        .iter()
        .copied()
        .find(|&p| !allies.contains(&p) && read::<i32>(p + OFF_MODE_D70) == 1);
    if let Some(p) = flagship {
        FLAGSHIP.store(p, Ordering::Relaxed);
        FLAGSHIP_STATE.store(read::<i32>(p + OFF_STATE), Ordering::Relaxed);
        FLAGSHIP_ALIVE.store(
            read::<i32>(p + OFF_ACTIVE) != 0 && !matches!(read::<i32>(p + OFF_STATE), 6 | 7),
            Ordering::Relaxed,
        );
        return;
    }
    let last = FLAGSHIP.load(Ordering::Relaxed);
    if last != 0 {
        FLAGSHIP.store(0, Ordering::Relaxed);
        let alive = FLAGSHIP_ALIVE.load(Ordering::Relaxed);
        // a retreat zeroes the retreat point; a shutdown or a loss leaves it as it was
        let used_point =
            read::<u64>(RETREAT_POINT) == 0 && RETREAT_POINT_SAVED.load(Ordering::Relaxed) != 0;
        log::info!(
            "ALLY flagship {last:#x} gone from the roster (last seen alive: {alive}, last state {}, retreat point used: {used_point})",
            FLAGSHIP_STATE.load(Ordering::Relaxed)
        );
        if alive {
            SUM_FLAGS.fetch_or(F_FLAGSHIP_LEFT, Ordering::Relaxed);
        }
        if alive && HANDOVER.load(Ordering::Relaxed) {
            SUM_FLAGS.fetch_or(F_HANDOVER, Ordering::Relaxed);
            let saved = RETREAT_POINT_SAVED.load(Ordering::Relaxed);
            let now: u64 = read(RETREAT_POINT);
            // only when somebody can still use it: an ally stays, or a reserve ship will take over
            let someone_left = !allies.is_empty() || read::<usize>(RESERVE + 8) > 0;
            if now == 0 && saved != 0 && someone_left {
                (RETREAT_POINT as *mut u64).write_unaligned(saved);
            }
            log::info!(
                "ALLY handover: flagship left, {} ally(s) stay and one is promoted; retreat point {:#x} -> {:#x} (saved {saved:#x})",
                allies.len(),
                now,
                read::<u64>(RETREAT_POINT),
            );
            diag::flush_log();
        } else if alive {
            // The manager promotes roster[0] this very frame (no mode-1 ship left), so a fly-out order comes
            // too late: extract the allies now, the way the manager does for strike craft at the edge.
            let extract: ExtractFn = std::mem::transmute(EXTRACT);
            let before = ships.len();
            for &ally in &allies {
                extract(ally, 0);
            }
            SUM_FLAGS.fetch_or(F_EXTRACTED, Ordering::Relaxed);
            // no replacement ally after a retreat of the whole squad
            ALLY_DONE.store(true, Ordering::Relaxed);
            let after = roster();
            log::info!(
                "ALLY retreat with the flagship: extracted {} ally(s), roster {before} -> {}, reserve {}, wave timer {}",
                allies.len(),
                after.len(),
                read::<usize>(RESERVE + 8),
                read::<f32>(0x1_47fc_7020),
            );
        }
    }
}

/// Logs the `ALLY SUMMARY` line of the battle that just ended (once). `now` = the game clock of the end.
unsafe fn emit_summary(now: f64) {
    if SUM_T0.load(Ordering::Relaxed) == 0 {
        return;
    }
    let t0 = f64::from_bits(SUM_T0.swap(0, Ordering::Relaxed));
    let flags = SUM_FLAGS.load(Ordering::Relaxed);
    let mut result = read::<i32>(BATTLE_RESULT);
    if result == 0 {
        result = SUM_RESULT.load(Ordering::Relaxed);
    }
    let state = SUM_ALLY_STATE.load(Ordering::Relaxed);
    let (honor_player, honor_allies) = honor_share::totals();
    let ally = if flags & F_SPAWNED == 0 {
        if flags & F_OFF != 0 {
            "none (switched off)"
        } else {
            "none (not brought)"
        }
        .to_string()
    } else if flags & F_EXTRACTED != 0 {
        "retreated with the flagship".to_string()
    } else if matches!(state, 6 | 7) {
        format!("destroyed (state {state})")
    } else if flags & F_HANDOVER != 0 {
        format!("promoted after the flagship left (last state {state})")
    } else {
        format!("alive at the end (last state {state})")
    };
    log::info!(
            "ALLY SUMMARY v{}: {:.0} s, squad {} (flagship + {} others), enemies at start {}, result {result} ({}), flagship left alive: {}, retreat key used: {}, allies brought {} lost {}, ally: {ally}, honor {honor_player:.1} (allies {honor_allies:.1})",
            env!("CARGO_PKG_VERSION"),
            now - t0,
            SUM_SQUAD.load(Ordering::Relaxed),
            SUM_SQUAD.load(Ordering::Relaxed).saturating_sub(1),
            SUM_ENEMIES.load(Ordering::Relaxed),
            match result {
                -1 => "squad gone, enemies remain",
                1 => "enemies gone",
                2 => "both gone",
                _ => "unknown",
            },
            flags & F_FLAGSHIP_LEFT != 0,
            flags & F_RETREAT_KEY != 0,
            SUM_SPAWNED.load(Ordering::Relaxed),
            SUM_LOST.load(Ordering::Relaxed),
        );
}

/// Collects the per-battle numbers and logs one `ALLY SUMMARY` line when the battle is over.
unsafe fn summary_tick(ships: &[usize]) {
    let now = read::<f64>(CLOCK);
    if !ships.is_empty() {
        if SUM_T0.load(Ordering::Relaxed) == 0 {
            SUM_T0.store(now.max(0.001).to_bits(), Ordering::Relaxed);
            SUM_ENEMIES.store(
                count(ENEMIES_BEGIN, ENEMIES_END, 0x18) as u32,
                Ordering::Relaxed,
            );
            SUM_SQUAD.store(
                (ships.len() + read::<usize>(RESERVE + 8).min(32)) as u32,
                Ordering::Relaxed,
            );
            SUM_ALLY_STATE.store(-1, Ordering::Relaxed);
            SUM_RESULT.store(0, Ordering::Relaxed);
            SUM_FLAGS.store(0, Ordering::Relaxed);
            honor_share::reset_totals();
        }
        SUM_LAST.store(now.to_bits(), Ordering::Relaxed);
        let result = read::<i32>(BATTLE_RESULT);
        if result != 0 {
            SUM_RESULT.store(result, Ordering::Relaxed);
        }
        let ally = ALLY_PTR.load(Ordering::Relaxed);
        if ally != 0 && ships.contains(&ally) {
            SUM_ALLY_STATE.store(read::<i32>(ally + OFF_STATE), Ordering::Relaxed);
        }
    } else {
        emit_summary(now);
    }
}

static SPARE_PRIZES: AtomicBool = AtomicBool::new(false);
static PRIZE_LINES: AtomicU32 = AtomicU32::new(0);
static LAST_PRIZE_KEY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Enemies (side < 0, in the level) that carry the prize flag.
unsafe fn prize_enemies() -> Vec<usize> {
    let (cb, ce): (usize, usize) = (read(CREATURES_BEGIN), read(CREATURES_END));
    (cb..ce)
        .step_by(0x18)
        .map(|a| read::<usize>(a + 8))
        .filter(|&p| {
            p != 0
                && read::<i32>(p + OFF_ACTIVE) != 0
                && read::<f32>(p + OFF_SIDE) < 0.0
                && read::<u8>(p + OFF_PRIZE) != 0
        })
        .collect()
}

/// The ship (creature) a target node belongs to: a creature (class code 47, `(code & 0x2f) == 0x2f` as the game's
/// own check in `FUN_140079810`) is its own ship; any other node (a part) points to its owning creature at +0xb8
/// (structures.md: "owner pointer"). The first version climbed the parent chain (+0x88) like the game's
/// `traverseLinkedListCreature_`, which gives the root PART of a chain, not the ship: it read the prize flag on a
/// wrong object and re-targeted every ally every tick (log of 2026-10-16, v0.9.0).
unsafe fn creature_of(node: usize) -> usize {
    if read::<u8>(node + OFF_CODE) & 0x2f == 0x2f {
        return node;
    }
    let owner: usize = read(node + 0xb8);
    if owner != 0 && read::<u8>(owner + OFF_CODE) & 0x2f == 0x2f {
        owner
    } else {
        node
    }
}

/// Runs right after the AI has chosen a target for `ship`. An AI-run ship of the player's side (an ally) that
/// picked a prize ship takes the nearest other enemy instead, or no target when only prizes are left.
unsafe extern "C" fn on_target_chosen(ship: usize) {
    let _hook = diag::scope("on_target_chosen");
    if !plausible(ship) || !SPARE_PRIZES.load(Ordering::Relaxed) {
        return;
    }
    // v0.9.3: only the ships this mod brought in (not the vanilla defenders of a missile or plane defense battle)
    if !ARMED.load(Ordering::Relaxed) {
        return;
    }
    match ALLIES.try_lock() {
        Ok(v) if v.contains(&ship) => {}
        _ => return,
    }
    // friendly and AI-run: the player's side, AI flag set (the ship the player flies has it cleared)
    if read::<f32>(ship + OFF_SIDE) <= 0.0 || read::<u8>(ship + OFF_AI_FLAG) == 0 {
        return;
    }
    let node: usize = read(ship + OFF_TARGET);
    if !plausible(node) {
        return;
    }
    // the target may be a part of an enemy ship: the prize flag is on the ship
    let target = creature_of(node);
    if !plausible(target) || read::<u8>(target + OFF_PRIZE) == 0 {
        return;
    }
    let (sx, sy) = (read::<f32>(ship + OFF_POS_X), read::<f32>(ship + OFF_POS_Y));
    let (cb, ce): (usize, usize) = (read(CREATURES_BEGIN), read(CREATURES_END));
    if cb < 0x10000 || ce < cb || (ce - cb) / LINK_STRIDE > CREATURE_LIST_LIMIT {
        return;
    }
    let mut best: Option<(f32, usize)> = None;
    for a in (cb..ce).step_by(LINK_STRIDE) {
        let p: usize = read(a + 8);
        if !plausible(p)
            || p == target
            || read::<i32>(p + OFF_ACTIVE) == 0
            || read::<f32>(p + OFF_SIDE) >= 0.0
            || read::<u8>(p + OFF_PRIZE) != 0
        {
            continue;
        }
        if matches!(read::<i32>(p + OFF_STATE), 6 | 7) {
            continue;
        }
        let (dx, dy) = (
            read::<f32>(p + OFF_POS_X) - sx,
            read::<f32>(p + OFF_POS_Y) - sy,
        );
        let d = dx * dx + dy * dy;
        if best.map_or(true, |(bd, _)| d < bd) {
            best = Some((d, p));
        }
    }
    let set_link: SetLinkFn = std::mem::transmute(SET_LINK);
    let replacement = best.map_or(0, |(_, p)| p);
    // the target may have been replaced or freed by now: only write while the ship still points at what we checked
    if !live_ship(ship) || read::<usize>(ship + OFF_TARGET) != node {
        return;
    }
    set_link(ship + OFF_TARGET_LINK, replacement);
    let key = (ship ^ target.rotate_left(17) ^ replacement.rotate_left(34)) as u64;
    if LAST_PRIZE_KEY.swap(key, Ordering::Relaxed) != key
        && PRIZE_LINES.fetch_add(1, Ordering::Relaxed) < 60
    {
        log::info!(
            "ALLY target: c={ship:#x} would attack the prize ship c={target:#x}; now {}",
            if replacement == 0 {
                "no target (only prizes left)".to_string()
            } else {
                format!("c={replacement:#x}")
            }
        );
    }
}

/// Called at the start of the per-frame battle manager.
unsafe extern "C" fn on_battle_tick() {
    let _hook = diag::scope("on_battle_tick");
    plane_loadouts::verify(read::<f64>(CLOCK));
    plane_loadouts::tick();
    precombat::tick(1);
    let ships = roster();
    restore_orders("battle tick", false);
    ship_switch::tick(&ships);
    if !ARMED.load(Ordering::Relaxed) {
        // not an ally battle: touch nothing; just keep the roster-empty bookkeeping so a later battle starts clean
        if ships.is_empty() {
            PLAYER_SEEN.store(0, Ordering::Relaxed);
        }
        return;
    }
    battle_diagnostics::tick(&ships);
    retreat_logic(&ships);
    // A manual handoff can introduce a new AI ally even after the reserve is exhausted.
    if ally_ammo::enabled() {
        if let Ok(allies) = ALLIES.lock() {
            ally_ammo::roll_new(&ships, &allies);
        }
    }
    ally_sound_events(&ships);
    summary_tick(&ships);
    if !ships.is_empty() {
        precombat::hide("battle running");
    }
    if ships.is_empty() {
        // between battles: arm again
        if ALLY_DONE.swap(false, Ordering::Relaxed) {
            log::info!("ALLY battle over, armed again");
            diag::flush_log();
        }
        PLAYER_SEEN.store(0, Ordering::Relaxed);
        return;
    }
    if ALLY_DONE.load(Ordering::Relaxed) {
        // nothing more to bring; allies already out are tracked above
        return;
    }
    // the ship the player flies: mode 1, in the level, not dead (state 6/7)
    let flying = ships.iter().any(|&p| {
        read::<i32>(p + OFF_MODE_D70) == 1
            && read::<i32>(p + OFF_ACTIVE) != 0
            && !matches!(read::<i32>(p + OFF_STATE), 6 | 7)
    });
    if !flying {
        return;
    }
    let now = read::<f64>(CLOCK);
    let seen = f64::from_bits(PLAYER_SEEN.load(Ordering::Relaxed));
    if seen == 0.0 {
        PLAYER_SEEN.store(now.to_bits(), Ordering::Relaxed);
        return;
    }
    let test_allies = crate::shipworks::test_allies();
    if test_allies == 0 && now - seen < f64::from_bits(ALLY_DELAY.load(Ordering::Relaxed)) {
        return;
    }

    // allies still fighting; the lost ones free a place for the next ship switched on
    let alive = {
        let Ok(mut allies) = ALLIES.lock() else {
            return;
        };
        allies.retain(|p| {
            let dead =
                matches!(read::<i32>(*p + OFF_STATE), 6 | 7) || read::<i32>(*p + OFF_ACTIVE) == 0;
            if ships.contains(p) && dead {
                SUM_LOST.fetch_add(1, Ordering::Relaxed);
                log::info!(
                    "ALLY lost: c={:#x} destroyed (state {})",
                    *p,
                    read::<i32>(*p + OFF_STATE)
                );
                diag::flush_log();
                return false;
            }
            ships.contains(p)
        });
        allies.len() as u32
    };
    let max_allies = if test_allies > 0 {
        test_allies
    } else {
        MAX_ALLIES.load(Ordering::Relaxed)
    };
    if alive >= max_allies {
        return;
    }
    // at most one deploy per 3 game seconds
    let last = f64::from_bits(LAST_DEPLOY.load(Ordering::Relaxed));
    if test_allies == 0 && last != 0.0 && now - last < 3.0 {
        return;
    }
    let reserve: usize = read(RESERVE + 8);
    if !BATTLE_LOGGED.swap(true, Ordering::Relaxed) {
        let (cb, ce): (usize, usize) = (read(CREATURES_BEGIN), read(CREATURES_END));
        let enemy = (cb..ce)
            .step_by(0x18)
            .map(|a| read::<usize>(a + 8))
            .find(|&p| p != 0 && read::<f32>(p + OFF_SIDE) < 0.0);
        log::info!(
            "ALLY battle t={now:.1}: roster {}, reserve {reserve}, prize enemies at the first tick {}, switched on {:#x}, max allies {}, enemy mode_d70={:?}",
            ships.len(),
            prize_enemies().len(),
            precombat::selected_mask(),
            max_allies,
            enemy.map(|p| read::<i32>(p + OFF_MODE_D70))
        );
    }
    if reserve == 0 || reserve > 32 {
        ALLY_DONE.store(true, Ordering::Relaxed);
        if SUM_SPAWNED.load(Ordering::Relaxed) == 0 {
            log::info!("ALLY no reserve ship to bring (reserve size {reserve})");
        } else {
            log::info!("ALLY no reserve ship left to bring in");
        }
        return;
    }

    // the next ship switched on that has not been brought yet; its place in the reserve is its number minus 2,
    // minus the lower-numbered ships already deployed
    let mask = precombat::selected_mask();
    let used = USED_MASK.load(Ordering::Relaxed);
    let candidate = (2..(2 + reserve + used.count_ones() as usize))
        .find(|n| *n <= 9 && mask & (1 << n) != 0 && used & (1 << n) == 0);
    let Some(slot) = candidate else {
        ALLY_DONE.store(true, Ordering::Relaxed);
        if SUM_SPAWNED.load(Ordering::Relaxed) == 0 {
            SUM_FLAGS.fetch_or(F_OFF, Ordering::Relaxed);
            log::info!(
                "ALLY off for this battle (no ship switched on; reserve {reserve}, mask {mask:#x})"
            );
        }
        return;
    };
    let lower_used = (used & ((1u32 << slot) - 1)).count_ones() as usize;
    let index = slot - 2 - lower_used.min(slot - 2);
    USED_MASK.fetch_or(1 << slot, Ordering::Relaxed);
    LAST_DEPLOY.store(now.to_bits(), Ordering::Relaxed);
    if index >= reserve {
        log::info!("ALLY ship number {slot}: reserve entry {index} does not exist (reserve {reserve}), skipped");
        return;
    }

    let deploy: DeployFn = std::mem::transmute(DEPLOY);
    let set_controlled: SetControlledFn = std::mem::transmute(SET_CONTROLLED);
    let set_link: SetLinkFn = std::mem::transmute(SET_LINK);
    let get_profile: GetProfileFn = std::mem::transmute(GET_PROFILE);

    let moved = reserve_to_front(index);
    log::info!(
        "ALLY chosen ship number {slot} (reserve entry {index}), moved to the front: {moved}"
    );
    let before = ships.len();
    let mut pos: u64 = 0;
    let ok = deploy(
        RESERVE,
        ROSTER_BEGIN,
        &mut pos,
        1,
        read::<f32>(ARENA_X_LOW) + 100.0,
        read::<f32>(ARENA_X_HIGH) - 100.0,
        read::<f32>(ARENA_Y_LOW) + 100.0,
        read::<f32>(DEPLOY_K),
        read::<f32>(DEPLOY_AGGR).to_bits(),
        ALLY_SLOT as i32,
    );
    let after = roster();
    log::info!(
        "ALLY deploy returned {ok}, roster {before} -> {}, reserve {reserve} -> {}",
        after.len(),
        read::<usize>(RESERVE + 8)
    );
    diag::flush_log();
    if after.len() != before + 1 {
        return; // nothing new to fix up
    }
    let Some(&ally) = after.last() else { return };

    // Record the ammo the reserve was funded with before the AI controller starts re-setting `+0x850`.
    let funded = ally_ammo::record_funding(ally);
    log::info!("ALLY ammo funding: c={ally:#x} recorded the funded ammo index of {funded} guns");

    // hand it back to the stock AI: not controlled (AI flag on), no control slot, wingman mode
    set_controlled(get_profile(), 0, ALLY_SLOT as u32);
    set_link(CONTROLLED_LINKS + ALLY_SLOT * 0x18, 0);
    (ally as *mut i32)
        .byte_add(OFF_MODE_D70)
        .write_unaligned(ALLY_MODE);
    if let Ok(mut allies) = ALLIES.lock() {
        allies.push(ally);
    }
    sound::joined();
    if SUM_SPAWNED.fetch_add(1, Ordering::Relaxed) == 0 {
        battle_diagnostics::start_tracking(ally);
        ALLY_PTR.store(ally, Ordering::Relaxed);
    }
    SUM_FLAGS.fetch_or(F_SPAWNED, Ordering::Relaxed);
    let (zn, gn) = battle_diagnostics::loadout_counts(ally);
    log::info!("ALLY loadout (ship {slot}): zenith launchers {zn}, guns {gn}");
    log::info!(
        "ALLY ready c={ally:#x} side={} ai_flag={} mode_d70={} state={} aggr={}",
        read::<f32>(ally + OFF_SIDE),
        read::<u8>(ally + OFF_AI_FLAG),
        read::<i32>(ally + OFF_MODE_D70),
        read::<i32>(ally + OFF_STATE),
        read::<f32>(ally + OFF_AGGRESSION),
    );
}
