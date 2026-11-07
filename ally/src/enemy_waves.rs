//! Enemy wave size.
//!
//! Normal tactical combat has no total enemy-ship cap: `SCR_COMBAT` uses the engagement list filled by
//! `Precombat_SplitFleetIntoReserveLists`. `Battle_DeployFromReserve` (`0x1402d22e0`) places up to 3 ships per
//! call (two `cmp rdx,3`) and stops when a weight budget of 6 runs out (a ship costs 1 to 3). The stock 30 s
//! delay before the first refill is left untouched.

use patchy::PatchSession;
const GROUP_CMP: [(usize, [u8; 4]); 2] = [(0x1_402d_237a, [0x48, 0x83, 0xfa, 0x03]), (0x1_402d_2c77, [0x48, 0x83, 0xfa, 0x03])];
const BUDGET_SITE: usize = 0x1_402d_23d9;
const BUDGET_BYTES: [u8; 6] = [0x41, 0xbc, 0x06, 0x00, 0x00, 0x00];

pub unsafe fn install(session: &mut PatchSession, group: u32) {
    let same = |addr: usize, want: &[u8]| std::slice::from_raw_parts(addr as *const u8, want.len()) == want;
    if group != 3 {
        let group = group.clamp(1, 40) as u8;
        if GROUP_CMP.iter().all(|(a, b)| same(*a, b)) && same(BUDGET_SITE, &BUDGET_BYTES) {
            let mut ok = true;
            for (addr, bytes) in GROUP_CMP {
                let mut patched = bytes;
                patched[3] = group;
                ok &= session.overwrite(addr, &patched).is_ok();
            }
            let mut budget = BUDGET_BYTES;
            budget[2..6].copy_from_slice(&(3 * group as u32).to_le_bytes());
            ok &= session.overwrite(BUDGET_SITE, &budget).is_ok();
            if ok {
                log::info!("hf_ally: enemy waves of up to {group} ships (weight budget {})", 3 * group as u32);
            } else {
                log::warn!("hf_ally: enemy_group_size failed");
            }
        } else {
            log::warn!("hf_ally: enemy_group_size not applied, the bytes at the wave sites differ");
        }
    }
}
