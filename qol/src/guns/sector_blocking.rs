use highfleet::{
    general::{CVec, Node},
    v1_163::Body,
};
use std::{
    f32::consts::TAU,
    mem::{align_of, offset_of, size_of},
};

const SECTOR_COUNT: usize = 360;
const CODE_BODY: u64 = 0x0f;
const MDL_SECTORS_TYPE_OFFSET: usize = 0x128;

const GET_STATS_ADDR: usize = 0x140281e00;

type GetStatsFn = unsafe extern "C" fn(i32) -> *const u8;

const _: () = {
    assert!(offset_of!(Body, base) == 0);
    assert!(offset_of!(Body, part_index) == 0x188);
    assert!(offset_of!(Body, angle) == 0x138);
    assert!(offset_of!(Body, sectors) == 0x508);
    assert!(size_of::<CVec<f32>>() == 3 * size_of::<*const f32>());
    assert!(align_of::<CVec<f32>>() == align_of::<*const f32>());
    assert!(offset_of!(Node, owner.target) == 0xb8);
    assert!(offset_of!(Node, master.target) == 0x88);
    assert!(offset_of!(Node, base.base.code) == 0x28);
};

/// Checks borrowed native state without adopting or dropping any game-owned fields.
/// FireGun passes a live, aligned Body on the game thread; its links and
/// sector storage remain stable for the duration of the callback.
pub(super) unsafe extern "C" fn is_gun_blocked(gun: *const Body) -> bool {
    if gun.is_null() {
        return false;
    }

    // SAFETY: Steam 1.163 FireGun uses this native GetStats function and Body layout.
    let get_stats: GetStatsFn = std::mem::transmute(GET_STATS_ADDR);
    let part = get_stats((*gun).part_index);
    if part.is_null() {
        return false;
    }
    // ShipPart is not modeled yet, so its sector-type flag remains an isolated raw access.
    if *part.add(MDL_SECTORS_TYPE_OFFSET).cast::<i32>() == 0 {
        return false;
    }

    let owner = (*gun).base.owner.target;
    if owner.is_null() || ((*owner).base.base.code & 0x0f) != CODE_BODY {
        return false;
    }

    // Borrow the CVec header's first two pointers without calling offset_from
    // on unknown native pointers. Only a complete, aligned 360-float span is used.
    let header = std::ptr::addr_of!((*gun).sectors).cast::<*const f32>();
    let begin = header.read();
    let end = header.add(1).read();
    if begin.is_null()
        || !begin.is_aligned()
        || (end as usize).checked_sub(begin as usize) != Some(SECTOR_COUNT * size_of::<f32>())
    {
        return false;
    }

    let mut root = owner;
    loop {
        let cursor = (*root).master.target;
        if cursor.is_null() || ((*cursor).base.base.code & 0x0f) != CODE_BODY {
            break;
        }
        root = cursor;
    }

    // Every accepted root has passed the Node code check before this Body cast.
    let mut rel = (*gun).angle - (*root.cast::<Body>()).angle;
    let steps = (rel.abs() / TAU + 0.5).floor();
    let full_turns = if rel >= 0.0 {
        steps as i32
    } else {
        -(steps as i32)
    };
    rel -= full_turns as f32 * TAU;
    if rel < 0.0 {
        rel += TAU;
    }

    let index = ((rel / TAU) * SECTOR_COUNT as f32) as usize;
    // SAFETY: Both links and the sector span are stable during FireGun. Read
    // only the selected scalar; no owned vector or Body reference is created.
    rel.is_finite() && rel >= 0.0 && index < SECTOR_COUNT && *begin.add(index) == 0.0
}
