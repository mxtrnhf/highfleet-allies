//! Steam 1.163's native `SCR_BOMBERS`, driven by a temporary Shipworks roster.
//! See docs/shipworks-test-allies.md for the state machine and native lifetimes.

use std::{
    ffi::CStr,
    mem::{offset_of, size_of},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        Mutex,
    },
};

use patchy_host::Condition;

use crate::patchy::{PatchSession, ReturnType, Trampoline};

use crate::{
    memory::{self, vector_span},
    native::{read, SetLinkFn, SET_LINK},
    native_layout::{Body, NODE_CODE},
};

use super::{
    super::{
        damage::{self, Scenario::Airstrike, Slot},
        prune_enemy_escadra, write, COPY_STRING, ENEMY_LIST, LINK_CLEAR_VECTOR, LINK_NODE_STRIDE,
        LINK_TARGET, LINK_UNLINK, NATIVE_NEW, SESSION, SIDE_ALLY, SIDE_ENEMY, TEST_FIGHT_FLAG,
    },
    Launch, ALL_PARTS, ENTRY_LIMIT, LEVEL, MAX_MULTIPLIER, MAX_PLANES,
};

const FRIENDLY_LIST: usize = 0x1_43a1_58e0;
const ESCADRA_CODE: u64 = 0x147;
// Escadra altitude read by Bombers' defender placement at 0x1402d2e61.
const FLEET_ALTITUDE: usize = 0x168;
const LOAD_LEVEL: usize = 0x1_4025_afd0;
const EDITOR: &CStr = c"Editor.seria";
const MAP_CRAFT_SIZE: usize = 0x340;
const MAP_CRAFT_CODE: u64 = 0x8000_0000_1047;
const MAP_CRAFT_CTOR: usize = 0x1_400e_4530;
const MAP_CRAFT_DTOR: usize = 0x1_400e_8030;
const ASSIGN_NODE_ID: usize = 0x1_4028_0840;
// Launched's native alignment setter at vtable +0x180 stores this side.
const MAP_CRAFT_SIDE: usize = 0x128;
const DEFINE_PART: usize = 0x1_4021_b190;

const BOMBERS_SETUP: usize = 0x1_402c_a2b0;
const BOMBERS_TEARDOWN: usize = 0x1_402c_b6a0;
const GET_PROFILE: usize = 0x1_4029_2a90;
const SCRIPTED_AIRCRAFT: usize = 0x1_47fc_6fdb;
const BOMBERS_STRATEGIC_RESULT: usize = 0x1_47fc_6fdf;
const BOMBERS_ACTIVE_PLANES: usize = 0x1_47fc_73b0;
const BOMBERS_DEFENDER_QUEUE: usize = 0x1_47fc_75a8;
// Filled by SCR_BOMBERS' spawner (0x1402a8fc0) and pruned by its tick.
pub(in crate::shipworks) const DEFENDER_ROSTER: usize = 0x1_47fc_73d8;
pub(in crate::shipworks) const DEFENDER_LIMIT: usize = 5;
const BOMBERS_AUX_LINKS: usize = 0x1_47fc_7590;
const LIST_LINK: usize = 0x10;
const LIST_NODE_SIZE: usize = LIST_LINK + LINK_NODE_STRIDE;
const NATIVE_DELETE: usize = 0x1_4033_40d0;
const PLANE_LIMIT: usize = ENTRY_LIMIT * MAX_PLANES as usize * MAX_MULTIPLIER as usize;

const NATIVE_WAVE_SPACING: usize = 0x1_4035_b93c;
const MAX_WAVE_SPACING: f32 = 100.0;
const MAX_WAVE_DEPTH: f32 = 3_500.0;
static WAVE_SPACING: AtomicU32 = AtomicU32::new(MAX_WAVE_SPACING.to_bits());

static REQUEST: Mutex<Option<Launch>> = Mutex::new(None);
static PENDING_SETUP: Mutex<Option<Launch>> = Mutex::new(None);
static ACTIVE: AtomicBool = AtomicBool::new(false);
// Set only for the MapCraft created below, not a game-owned map group.
static OWN_GROUP: AtomicUsize = AtomicUsize::new(0);

type LoadLevel = unsafe extern "C" fn(*const i8, u8, u8);

pub(super) fn request(launch: Launch) -> bool {
    let Ok(mut request) = REQUEST.try_lock() else {
        return false;
    };
    *request = Some(launch);
    true
}

pub(in crate::shipworks) fn requested() -> bool {
    REQUEST.try_lock().is_ok_and(|request| request.is_some())
}

pub(in crate::shipworks) fn cancel_request() {
    if let Ok(mut request) = REQUEST.try_lock() {
        *request = None;
    }
}

pub(in crate::shipworks) fn cancel_unhandled_request() {
    if let Ok(mut request) = REQUEST.try_lock() {
        if request.take().is_some() {
            log::warn!("ACES: Shipworks Airstrike FIGHT did not accept the editor ship");
        }
    }
}

pub(in crate::shipworks) fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Called at FIGHT's level load, with the native scratch profile and both
/// fleets initialized, after validation of the editor design.
pub(in crate::shipworks) unsafe fn load_fight(resume: u8, flag: u8) {
    let Some(mut launch) = REQUEST.try_lock().ok().and_then(|mut slot| slot.take()) else {
        return;
    };
    let load: LoadLevel = std::mem::transmute(LOAD_LEVEL);
    match stage_defenders(&mut launch) {
        Ok(()) => {
            if let Ok(mut pending) = PENDING_SETUP.try_lock() {
                *pending = Some(launch);
                ACTIVE.store(true, Ordering::Relaxed);
                drop(pending);
                load(LEVEL.as_ptr(), resume, flag);
                return;
            }
        }
        Err(reason) => log::warn!("ACES: Shipworks Airstrike could not start: {reason}"),
    }
    prune_enemy_escadra();
    load(EDITOR.as_ptr(), 0, 0);
}

unsafe fn fleet_link(list: usize) -> Option<(usize, usize)> {
    let (link, _) = vector_span(list, LINK_NODE_STRIDE, 1).filter(|&(_, count)| count == 1)?;
    let fleet = read::<usize>(link + LINK_TARGET);
    (memory::rd::<u64>(fleet.wrapping_add(NODE_CODE)) == Some(ESCADRA_CODE))
        .then_some((link, fleet))
}

unsafe fn stage_defenders(launch: &mut Launch) -> Result<(), &'static str> {
    let (_, editor_fleet) = fleet_link(FRIENDLY_LIST).ok_or("missing Shipworks editor fleet")?;
    let (enemy_link, enemy_fleet) =
        fleet_link(ENEMY_LIST).ok_or("missing Shipworks enemy fleet")?;
    let (fleet, side, entries, slot) = {
        let state = SESSION.try_lock().map_err(|_| "Shipworks roster is busy")?;
        if launch.friendly {
            (
                enemy_fleet,
                SIDE_ENEMY,
                state.enemies.entries.clone(),
                Slot::Enemy as fn(usize) -> Slot,
            )
        } else {
            (
                editor_fleet,
                SIDE_ALLY,
                state.allies.entries.clone(),
                Slot::Ally as fn(usize) -> Slot,
            )
        }
    };
    if !launch.friendly {
        if let Some(ship) = damage::stage_editor(fleet, &launch.replay)? {
            launch.staged.push((ship, Slot::Editor));
        }
    }
    damage::stage_entries(
        fleet,
        side,
        &entries,
        slot,
        &launch.replay,
        &mut launch.staged,
    );
    if launch.staged.is_empty() {
        return Err("no surviving defender could be loaded");
    }
    // SAFETY: these links are the native FIGHT builder's live, single-element
    // vectors. Their targets are scratch-profile fleets, not campaign objects.
    // Bombers wants the defending fleet in B regardless of actual alignment.
    let set_link: SetLinkFn = std::mem::transmute(SET_LINK);
    set_link(enemy_link, fleet);
    log::info!(
        "ACES: Shipworks Airstrike staged {} defenders, {} strike flights ({}, {})",
        launch.staged.len(),
        launch.flights.len(),
        if launch.friendly { "friendly" } else { "enemy" },
        if launch.ground { "ground" } else { "airborne" }
    );
    Ok(())
}

/// Install at SCR_BOMBERS call sites, not the shared campaign scene helpers.
///
/// # Safety
/// Called only during initialization against the verified Steam 1.163 image.
pub(in crate::shipworks) unsafe fn install(session: &mut PatchSession) -> Result<(), String> {
    for (site, handler, result) in [
        (0x1_402c_5540, setup as *const (), ReturnType::Rax),
        (0x1_402c_5555, exit as *const (), ReturnType::None),
        (0x1_402c_5340, teardown as *const (), ReturnType::None),
    ] {
        let mut call = Trampoline::new();
        call.preserved_call(handler, &[], &[], result);
        call.absolute_jump(site + 5);
        session
            .detour_trampoline(site, 5, call)
            .map_err(|err| err.to_string())?;
    }

    // State 2: skip profile tutorial flags and go straight to the native raid tick.
    let mut tick = Trampoline::new();
    let campaign = tick.new_label();
    tick.preserved_call(is_active as *const (), &[], &[], ReturnType::Rax);
    tick.bytes(&[0x84, 0xc0]); // TEST AL, AL
    tick.jump_if(Condition::Equal, campaign);
    tick.absolute_jump(0x1_402c_552e);
    tick.bind(campaign).map_err(|err| err.to_string())?;
    tick.absolute_call(GET_PROFILE as *const ());
    tick.absolute_jump(0x1_402c_535c);
    session
        .detour_trampoline(0x1_402c_5357, 5, tick)
        .map_err(|err| err.to_string())?;

    // Native groups of three normally start another 2,500 units out
    // (ADDSS XMM7,XMM15 at 0x1402cb4a3). Keep the full test raid within reach.
    let mut spacing = Trampoline::new();
    spacing.preserved_call(wave_spacing as *const (), &[], &[], ReturnType::Rax);
    spacing.bytes(&[0x66, 0x44, 0x0f, 0x6e, 0xf8]); // MOVD XMM15,EAX; RAX is dead.
    spacing.absolute_jump(0x1_402c_b38f);
    session
        .detour_trampoline(0x1_402c_b386, 9, spacing)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

extern "C" fn is_active() -> bool {
    active()
}

unsafe extern "C" fn wave_spacing() -> u32 {
    if active() {
        WAVE_SPACING.load(Ordering::Relaxed)
    } else {
        read(NATIVE_WAVE_SPACING)
    }
}

unsafe extern "C" fn setup() -> u8 {
    let stock: unsafe extern "C" fn() -> u8 = std::mem::transmute(BOMBERS_SETUP);
    if !active() {
        return stock();
    }
    let Some(launch) = PENDING_SETUP
        .try_lock()
        .ok()
        .and_then(|mut slot| slot.take())
    else {
        return 0;
    };
    // SAFETY: state 1 is after Bombers loaded its world and before any simulation
    // consumes the fleets. Side A still contains a single native test FIGHT link.
    let Some((link, _)) = fleet_link(FRIENDLY_LIST) else {
        return 0;
    };
    let Some((_, defenders)) = fleet_link(ENEMY_LIST) else {
        return 0;
    };
    let Some(altitude) = memory::rd::<f32>(defenders + FLEET_ALTITUDE) else {
        return 0;
    };
    let Some((group, count)) = make_group(&launch) else {
        return 0;
    };
    let set_link: SetLinkFn = std::mem::transmute(SET_LINK);
    set_link(link, group);
    OWN_GROUP.store(group, Ordering::Relaxed);
    let gaps = count.div_ceil(3).saturating_sub(1).max(1);
    let spacing = MAX_WAVE_SPACING.min(MAX_WAVE_DEPTH / gaps as f32);
    WAVE_SPACING.store(spacing.to_bits(), Ordering::Relaxed);
    write(TEST_FIGHT_FLAG, 1u8);
    write(
        defenders + FLEET_ALTITUDE,
        if launch.ground { 0.0f32 } else { 1.0f32 },
    );
    let started = stock();
    write(defenders + FLEET_ALTITUDE, altitude);
    if started != 0 {
        if let Some(roster) = Airstrike.combat_roster() {
            damage::begin(
                Airstrike,
                &roster,
                &launch.staged,
                launch.preserve,
                launch.replay,
            );
        }
    }
    log::info!(
        "ACES: Shipworks Airstrike handed {count} planes to native Bombers (started={started})"
    );
    started
}

/// Allocate a private native MapCraft; it is never inserted into Profile.mapCrafts.
unsafe fn make_group(launch: &Launch) -> Option<(usize, usize)> {
    let library = read::<usize>(ALL_PARTS);
    if !memory::readable_span(library, size_of::<usize>()) {
        return None;
    }
    let new: unsafe extern "C" fn(usize) -> usize = std::mem::transmute(NATIVE_NEW);
    let ctor: unsafe extern "C" fn(usize) -> usize = std::mem::transmute(MAP_CRAFT_CTOR);
    let assign_id: unsafe extern "C" fn(usize, u8) = std::mem::transmute(ASSIGN_NODE_ID);
    let define: unsafe extern "C" fn(usize, *const i8, usize, i32) -> usize =
        std::mem::transmute(DEFINE_PART);
    let assign_string: unsafe extern "C" fn(usize, *const i8, usize) -> usize =
        std::mem::transmute(COPY_STRING);

    let storage = new(MAP_CRAFT_SIZE);
    if storage == 0 {
        return None;
    }
    // SAFETY: same native allocation and constructor as DynNewSerializable.
    // Parts stay attached to the group until the game's bomber setup detaches
    // them into the live scene. Only the private MapCraft is manually destroyed.
    let group = ctor(storage);
    assign_id(group, 0);
    write(
        group + MAP_CRAFT_SIDE,
        if launch.friendly {
            SIDE_ALLY
        } else {
            SIDE_ENEMY
        },
    );
    let mut count = 0;
    for flight in &launch.flights {
        for _ in 0..flight.count {
            let plane = define(library, flight.craft.as_ptr(), group, 1);
            if plane == 0 {
                log::warn!(
                    "ACES: Shipworks Airstrike could not define {}",
                    flight.craft.to_string_lossy()
                );
                break;
            }
            assign_string(
                plane + offset_of!(Body, logic_s1),
                flight.loadout.as_ptr(),
                flight.loadout.to_bytes().len(),
            );
            count += 1;
        }
    }
    if count == 0 {
        destroy_group(group);
        return None;
    }
    Some((group, count))
}

unsafe extern "C" fn exit(path: *const i8, resume: u8, flag: u8) {
    let load: LoadLevel = std::mem::transmute(LOAD_LEVEL);
    if active() {
        load(EDITOR.as_ptr(), 0, 0);
    } else {
        load(path, resume, flag);
    }
}

unsafe extern "C" fn teardown() {
    if !active() {
        let stock: unsafe extern "C" fn() = std::mem::transmute(BOMBERS_TEARDOWN);
        stock();
        return;
    }
    // Save while the spawned ships are still live and before the shared
    // level-exit call schedules Editor and releases our scratch fleet links.
    if let Some(roster) = Airstrike.combat_roster() {
        damage::finish(&roster);
    } else {
        damage::invalidate_replay(Airstrike);
    }
    // The shared level-exit call schedules Editor; leave() releases our staging.
}

/// Release the test scene's links immediately before the engine leaves Bombers.
/// The world unload still owns the tactical planes and defending ships.
pub(in crate::shipworks) unsafe fn leave() {
    cancel_request();
    if !ACTIVE.swap(false, Ordering::Relaxed) {
        return;
    }
    if let Ok(mut pending) = PENDING_SETUP.try_lock() {
        *pending = None;
    }
    damage::discard_encounter();
    let group = OWN_GROUP.swap(0, Ordering::Relaxed);
    let clear: unsafe extern "C" fn(usize) = std::mem::transmute(LINK_CLEAR_VECTOR);
    if group != 0 {
        // SAFETY: these globals were filled by the native Bombers setup for
        // this test scene. Destroy only their native link records, not targets.
        clear_list(BOMBERS_ACTIVE_PLANES, PLANE_LIMIT);
        clear_list(BOMBERS_DEFENDER_QUEUE, DEFENDER_LIMIT);
        clear(DEFENDER_ROSTER);
        clear(BOMBERS_AUX_LINKS);
        write(SCRIPTED_AIRCRAFT, 0u8);
        write(BOMBERS_STRATEGIC_RESULT, 0u8);
    }
    clear(FRIENDLY_LIST);
    clear(ENEMY_LIST);
    if group != 0 && memory::rd::<u64>(group + NODE_CODE) == Some(MAP_CRAFT_CODE) {
        destroy_group(group);
    }
    prune_enemy_escadra();
    log::info!("ACES: Shipworks Airstrike ended; cleared staging links");
}

unsafe fn destroy_group(group: usize) {
    let destroy: unsafe extern "C" fn(usize, u32) -> usize = std::mem::transmute(MAP_CRAFT_DTOR);
    destroy(group, 1);
}

unsafe fn clear_list(list: usize, limit: usize) {
    // Validate the bounded sentinel walk before unlinking anything. Native list
    // cells contain one Link at +0x10; the sentinel remains owned by the game.
    let (head, count) = (read::<usize>(list), read::<usize>(list + 8));
    if count > limit || !memory::readable_span(head, 2 * size_of::<usize>()) {
        return;
    }
    let mut cells = Vec::with_capacity(count);
    let mut cell = read::<usize>(head);
    let mut last = head;
    for _ in 0..count {
        if cell == head
            || !memory::readable_span(cell, LIST_NODE_SIZE)
            || read::<usize>(cell + 8) != last
        {
            return;
        }
        cells.push(cell);
        (last, cell) = (cell, read(cell));
    }
    if cell != head || read::<usize>(head + 8) != last {
        return;
    }
    let unlink: unsafe extern "C" fn(usize) = std::mem::transmute(LINK_UNLINK);
    let delete: unsafe extern "C" fn(usize, usize) = std::mem::transmute(NATIVE_DELETE);
    for node in cells {
        unlink(node + LIST_LINK);
        delete(node, LIST_NODE_SIZE);
    }
    write(head, head);
    write(head + 8, head);
    write(list + 8, 0usize);
}
