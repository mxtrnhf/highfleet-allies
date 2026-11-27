//! Enemy wave size.
//!
//! Normal tactical combat has no total enemy-ship cap: `SCR_COMBAT` uses the engagement list filled by
//! `Precombat_SplitFleetIntoReserveLists`. `Battle_DeployFromReserve` (`0x1402d22e0`) places up to 3 ships per
//! call (two `cmp rdx,3`) and stops when a weight budget of 6 runs out (a ship costs 1 to 3). The stock 30 s
//! delay before the first refill is left untouched.

use crate::patchy::PatchSession;

const GROUP_CMP: [(usize, [u8; 4]); 2] = [
    (0x1_402d_237a, [0x48, 0x83, 0xfa, 0x03]),
    (0x1_402d_2c77, [0x48, 0x83, 0xfa, 0x03]),
];
const BUDGET_SITE: usize = 0x1_402d_23d9;
const BUDGET_BYTES: [u8; 6] = [0x41, 0xbc, 0x06, 0x00, 0x00, 0x00];
const STOCK_GROUP_SIZE: u32 = 3;
const MAX_GROUP_SIZE: u32 = 40;
const WEIGHT_PER_SLOT: u32 = 3;

/// Stages the three edits that control the wave count and weight budget.
///
/// # Safety
/// Call during serialized initialization for Steam 1.163, while the fixed code
/// ranges above are mapped and no other thread is modifying them.
pub unsafe fn install(session: &mut PatchSession, group: u32) {
    if group == STOCK_GROUP_SIZE {
        return;
    }

    let group = group.clamp(1, MAX_GROUP_SIZE) as u8;
    let matches_code = |address: usize, expected: &[u8]| {
        std::slice::from_raw_parts(address as *const u8, expected.len()) == expected
    };
    if !GROUP_CMP
        .iter()
        .all(|(address, bytes)| matches_code(*address, bytes))
        || !matches_code(BUDGET_SITE, &BUDGET_BYTES)
    {
        log::warn!("hf_ally: enemy_group_size not applied, the bytes at the wave sites differ");
        return;
    }

    // Attempt every overwrite, even if an earlier one could not be staged.
    let mut staged_all = true;
    for (address, mut bytes) in GROUP_CMP {
        bytes[3] = group;
        staged_all &= session.overwrite(address, &bytes).is_ok();
    }

    let weight_budget = WEIGHT_PER_SLOT * u32::from(group);
    let mut budget_bytes = BUDGET_BYTES;
    budget_bytes[2..6].copy_from_slice(&weight_budget.to_le_bytes());
    staged_all &= session.overwrite(BUDGET_SITE, &budget_bytes).is_ok();

    if staged_all {
        log::info!("hf_ally: enemy waves of up to {group} ships (weight budget {weight_budget})");
    } else {
        log::warn!("hf_ally: enemy_group_size failed");
    }
}
