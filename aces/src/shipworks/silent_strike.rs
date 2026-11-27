//! Shipworks Silent Strike uses the native Combat scene and its campaign ground deployment.
//! See [the Shipworks encounter notes](../../../docs/shipworks-test-allies.md#silent-strike-steam-1163).

use std::{
    ffi::CStr,
    sync::atomic::{AtomicBool, Ordering},
};

use patchy_host::Condition;

use crate::patchy::{PatchSession, ReturnType, Trampoline};

use crate::{
    memory,
    native::{read, OFF_SIDE},
};

use super::TEST_FIGHT_FLAG;

pub(super) const BUTTON: &CStr = c"SW_ACE_SILENT";

// Request lasts only through synchronous FIGHT validation. Active lasts through pre-combat
// and every spawn in that Combat scene, but never across a return to Shipworks or the map.
static REQUESTED: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicBool = AtomicBool::new(false);

pub(super) fn request() {
    REQUESTED.store(true, Ordering::Relaxed);
}

pub(super) fn requested() -> bool {
    REQUESTED.load(Ordering::Relaxed)
}

pub(super) fn cancel_request() {
    REQUESTED.store(false, Ordering::Relaxed);
}

pub(super) fn cancel_unhandled_request() {
    if REQUESTED.swap(false, Ordering::Relaxed) {
        log::warn!("ACES: Shipworks Silent Strike FIGHT did not accept the editor ship");
    }
}

pub(super) fn begin_combat() {
    let active = REQUESTED.swap(false, Ordering::Relaxed);
    ACTIVE.store(active, Ordering::Relaxed);
    if active {
        log::info!("ACES: Shipworks Silent Strike entering native pre-combat");
    }
}

pub(super) fn leave() {
    cancel_request();
    ACTIVE.store(false, Ordering::Relaxed);
}

/// Route enemy ships to the stock landed-placement branch. The spawners still
/// find room on the terrain, initialize landing gear and handle reserve/wave timing.
///
/// # Safety
/// Install only during serialized initialization of the verified Steam 1.163 image.
pub(super) unsafe fn install(session: &mut PatchSession) -> Result<(), String> {
    for (site, landed, native_fleet_load, callback_arg) in [
        // Battle_DeployFromReserve: R13 is the template, R15B is its airborne flag.
        (
            0x1_402d_2498,
            0x1_402d_24bf,
            [0x49, 0x8b, 0x8d, 0x70, 0x0a, 0x00, 0x00], // MOV RCX,[R13+0xa70]
            [0x49, 0x8b, 0xcd],                         // MOV RCX,R13
        ),
        // BattleSpawner_SpawnShips: RSI is the template, R12B is its airborne flag.
        (
            0x1_402d_1bc2,
            0x1_402d_1bf5,
            [0x48, 0x8b, 0x8e, 0x70, 0x0a, 0x00, 0x00], // MOV RCX,[RSI+0xa70]
            [0x48, 0x8b, 0xce],                         // MOV RCX,RSI
        ),
    ] {
        let mut spawn = Trampoline::new();
        let on_ground = spawn.new_label();
        spawn.preserved_call(land_enemy as *const (), &callback_arg, &[], ReturnType::Rax);
        spawn.bytes(&[0x84, 0xc0]); // TEST AL,AL (the original float comparisons set flags again).
        spawn.jump_if(Condition::NotEqual, on_ground);
        spawn.bytes(&native_fleet_load);
        spawn.absolute_jump(site + native_fleet_load.len());
        spawn.bind(on_ground).map_err(|err| err.to_string())?;
        spawn.absolute_jump(landed);
        session
            .detour_trampoline(site, native_fleet_load.len(), spawn)
            .map_err(|err| err.to_string())?;
    }
    Ok(())
}

unsafe extern "C" fn land_enemy(template: usize) -> bool {
    // SAFETY: each spawner just resolved and checked this borrowed Creature template
    // before CalculateStats. On the game thread we read only its scalar side field
    // and neither retain the pointer nor modify the ship or fleet.
    ACTIVE.load(Ordering::Relaxed)
        && read::<u8>(TEST_FIGHT_FLAG) != 0
        && memory::rd::<f32>(template + OFF_SIDE).is_some_and(|side| side < 0.0)
}
