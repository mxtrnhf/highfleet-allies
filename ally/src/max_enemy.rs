//! `max_enemy_ships`: the battle set-up copies the engagement's enemy ships into the battle list only while the list
//! holds fewer than 5 (`cmp qword [0x147fc75b0], 5` at `0x1402caaa0`). The limit is the immediate byte.
//! Notes: `hf_notes/docs/findings/battle-ally.md` (battle set-up). The pre-combat screen has room for few enemy cards.

use patchy::PatchSession;

const SITE: usize = 0x1_402c_aaa0;
const BYTES: [u8; 8] = [0x48, 0x83, 0x3d, 0x08, 0xcb, 0xcf, 0x07, 0x05];

pub unsafe fn install(session: &mut PatchSession, limit: u32) {
    if limit == 5 {
        return;
    }
    let found = std::slice::from_raw_parts(SITE as *const u8, BYTES.len());
    if found != BYTES {
        log::warn!("hf_ally: max_enemy_ships not applied, bytes at {SITE:#x} are {found:02x?}");
        return;
    }
    let limit = limit.clamp(1, 127) as u8;
    let mut patched = BYTES;
    patched[7] = limit;
    match session.overwrite(SITE, &patched) {
        Ok(_) => log::info!("hf_ally: at most {limit} enemy ships per battle"),
        Err(e) => log::warn!("hf_ally: max_enemy_ships failed: {e}"),
    }
}

/// Enemy waves (notes `battle-ally.md`, "Enemy waves"): `0x1402d22e0` places at most 3 ships per call (two `cmp rdx,3`) and
/// stops when a weight budget of 6 runs out (a ship costs 1 to 3); the first wave waits for a 30.0 timer stored at
/// `0x1402cb579` (its load at `0x1402cb571` reads a constant shared with other code, so the load is replaced instead).
const GROUP_CMP: [(usize, [u8; 4]); 2] = [(0x1_402d_237a, [0x48, 0x83, 0xfa, 0x03]), (0x1_402d_2c77, [0x48, 0x83, 0xfa, 0x03])];
const BUDGET_SITE: usize = 0x1_402d_23d9;
const BUDGET_BYTES: [u8; 6] = [0x41, 0xbc, 0x06, 0x00, 0x00, 0x00];
const TIMER_LOAD: usize = 0x1_402c_b571;
const TIMER_LOAD_BYTES: [u8; 8] = [0xf3, 0x0f, 0x10, 0x05, 0x63, 0x6f, 0x08, 0x00];

pub unsafe fn install_waves(session: &mut PatchSession, group: u32, at_once: bool) {
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
    if at_once {
        if same(TIMER_LOAD, &TIMER_LOAD_BYTES) {
            // xorps xmm0,xmm0 ; nop5: the timer starts at 0.0
            match session.overwrite(TIMER_LOAD, &[0x0f, 0x57, 0xc0, 0x0f, 0x1f, 0x44, 0x00, 0x00]) {
                Ok(_) => log::info!("hf_ally: the first enemy wave starts at once"),
                Err(e) => log::warn!("hf_ally: enemy_all_at_once failed: {e}"),
            }
        } else {
            log::warn!("hf_ally: enemy_all_at_once not applied, the bytes at {TIMER_LOAD:#x} differ");
        }
    }
}
