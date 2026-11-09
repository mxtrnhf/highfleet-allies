//! Neutral, shared view of the Steam 1.163 process image.
//!
//! This module owns only native addresses, offsets, function signatures and the guarded
//! scalar/traversal helpers that more than one feature needs. Feature-owned hook sites,
//! prologues and state stay with their owner:
//!
//! * pre-combat / GUI sites and their prologues belong to `precombat`;
//! * spawn and Creature logging hooks belong to `lifecycle_log`;
//! * battle-manager, deploy, extract and prize-target hooks belong to `battle`, while
//!   `bootstrap` coordinates the one patch session;
//! * sound playback, logging and wall-clock helpers stay with their respective owners.
//!
//! # Borrowed game memory
//!
//! Every address here points into the live game process. Nothing in this module creates an
//! owning Rust value: pointers are plain `usize`, reads are unaligned scalar reads, and
//! [`roster`] returns a transient list of raw creature pointers.
//!
//! [`plausible`] and [`live_ship`] are range/class/state heuristics only. They do **not** prove
//! that a pointer is still allocated, is the object the caller expects, or remains valid after
//! the next native call. In particular:
//!
//! * do not create `&Creature`, `&Body`, `&Node` or any other reference with an inferred lifetime
//!   from these addresses;
//! * do not use `ptr::read`, `read_unaligned` on an owning type, or `from_raw_parts` to copy a
//!   game-owned string/container/object;
//! * never drop or free a game-owned allocation;
//! * take a fresh snapshot inside the same game-thread callback that uses it, and revalidate a
//!   pointer immediately before writing through it.
//!
//! [`read`] only performs the load; it does not check that the page is mapped. Use the
//! diagnostics page-checked reader when the address is not already known-good. The game-thread
//! rule applies to all traversal helpers; the diagnostics watchdog may read scalar globals only.

use std::mem::{offset_of, size_of};

use crate::native_layout::{Creature, Link, Node, NODE_CODE};

// --- image and scalar game globals -----------------------------------------------------------

/// Preferred image base of the supported Steam 1.163 executable.
pub(crate) const IMAGE_BASE: usize = 0x1_4000_0000;

/// Pointer to the global GUI node every GUI is added to; shared by pre-combat and sound.
pub(crate) const GUI_ROOT: usize = 0x1_47fc_6f88;

/// Game clock, `f64` seconds.
pub(crate) const CLOCK: usize = 0x1_43a1_19c0;

/// Battle result code, `i32`; read by the summary and the diagnostics watchdog.
pub(crate) const BATTLE_RESULT: usize = 0x1_47fc_7004;

/// Reserve list of ships waiting to deploy (`std::list`: sentinel at +0, size at +8).
pub(crate) const RESERVE: usize = 0x1_47fc_73b0;

/// Battle roster vector header: begin / end of `Link` entries (0x18 bytes, pointer at +8).
pub(crate) const ROSTER_BEGIN: usize = 0x1_47fc_7590;
pub(crate) const ROSTER_END: usize = 0x1_47fc_7598;

/// Scenario state selector, `u32`.
pub(crate) const SCENE_SELECTOR: usize = 0x1_47fc_6fc4;

/// Scene flags (campaign / battle / pending), read by the diagnostics watchdog.
pub(crate) const SCENE_FLAGS: [usize; 3] = [0x1_47fc_6fd0, 0x1_47fc_6fda, 0x1_43a1_1902];

/// Enemy creature list (`Link` entries): begin / end.
pub(crate) const ENEMIES_BEGIN: usize = 0x1_47fc_73d8;
pub(crate) const ENEMIES_END: usize = 0x1_47fc_73e0;

/// Controlled-ship links, one per control slot (0x18 bytes each).
pub(crate) const CONTROLLED_LINKS: usize = 0x1_43a1_c260;

/// The stock name for the first controlled link (`CONTROLLED_LINKS`); kept because existing
/// readers use the singular spelling.
pub(crate) const CONTROLLED_LINK: usize = CONTROLLED_LINKS;

/// Global creature list (`Link` entries, 0x18 bytes, pointer at +8): begin / end.
pub(crate) const CREATURES_BEGIN: usize = 0x1_43fa_a5d8;
pub(crate) const CREATURES_END: usize = 0x1_43fa_a5e0;

// --- shared native functions ----------------------------------------------------------------

/// `SET_LINK(link, node)`: native smart-link assignment.
pub(crate) const SET_LINK: usize = 0x1_4019_4de0;
pub(crate) const SET_LINK_PROLOGUE: [u8; 10] =
    [0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20];

/// `SET_CONTROLLED(profile, ship, slot)`: sets the AI flag from the control state.
pub(crate) const SET_CONTROLLED: usize = 0x1_402a_7e30;
pub(crate) const SET_CONTROLLED_PROLOGUE: [u8; 10] =
    [0x48, 0x89, 0x5c, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20];

/// `GET_PROFILE()`: the current player profile.
pub(crate) const GET_PROFILE: usize = 0x1_4029_2a90;
pub(crate) const GET_PROFILE_PROLOGUE: [u8; 8] = [0x0f, 0xb6, 0x15, 0xea, 0x44, 0xd3, 0x07, 0x0f];

pub(crate) type SetLinkFn = unsafe extern "C" fn(usize, usize);
pub(crate) type SetControlledFn = unsafe extern "C" fn(usize, usize, u32) -> usize;
pub(crate) type GetProfileFn = unsafe extern "C" fn() -> usize;

// --- shared limits ---------------------------------------------------------------------------

/// Byte stride of one `Link` entry in the native object vectors: the pointer sits at +8.
pub(crate) const LINK_STRIDE: usize = size_of::<Link>();

/// Ceiling on the entries read from one battle roster / ship part / gun list. A real ship has far
/// fewer, so a larger count means the native vector header is corrupt.
pub(crate) const SHIP_LIST_LIMIT: usize = 64;

/// Ceiling on the entries read from the global creature list, for the same reason.
pub(crate) const CREATURE_LIST_LIMIT: usize = 4096;

// --- shared creature / node / body offsets ---------------------------------------------------

/// Template or node class code (`u64`); the low bits identify a creature node.
pub(crate) const OFF_CODE: usize = NODE_CODE;

/// `i32` order role: 1 = flown by the player, 2 = strike/retreat, 0 = plain AI.
pub(crate) const OFF_MODE_D70: usize = offset_of!(Creature, order_role);

/// Node state, `i32`; non-zero while the ship is in the level.
pub(crate) const OFF_ACTIVE: usize = offset_of!(Node, state);

/// Side, `f32`: negative = AI, positive = player, 0 = neutral.
pub(crate) const OFF_SIDE: usize = offset_of!(Creature, side);

/// AI flag, `u8`: non-zero = joins the AI vector when added.
pub(crate) const OFF_AI_FLAG: usize = offset_of!(Creature, ai_flag);

/// Aggression, `f32`: >= 0.5 makes the driver call the aggressive chase entry.
pub(crate) const OFF_AGGRESSION: usize = 0xbc4;

/// Driver state, `i32`; 1/2 run the AI tick, 6/7 are dead.
pub(crate) const OFF_STATE: usize = offset_of!(Creature, driver_state);

/// Mode, `i32`.
pub(crate) const OFF_MODE: usize = offset_of!(Creature, mode);

/// Current target pointer: the `Link` target whose node sits at `+0xb00`.
pub(crate) const OFF_TARGET: usize = offset_of!(Creature, target) + offset_of!(Link, target);

/// The `Link` whose node is the current target (`Creature::target`).
pub(crate) const OFF_TARGET_LINK: usize = offset_of!(Creature, target);

/// Prize flag, `u8`: the ship may be captured after the battle.
pub(crate) const OFF_PRIZE: usize = 0xc19;

/// Creature centre, `f32`.
pub(crate) const OFF_POS_X: usize = offset_of!(Creature, bounds_center_x);
pub(crate) const OFF_POS_Y: usize = offset_of!(Creature, bounds_center_y);

/// Five control bytes: group 0, group 1, bombs, spare, flares.
pub(crate) const OFF_FLAGS: usize = offset_of!(Creature, trigger_gun_group_0);

// --- guarded scalar reads and transient traversal --------------------------------------------

/// Reads `T` at `address` without requiring alignment.
///
/// # Safety
///
/// The caller must establish that `size_of::<T>()` bytes starting at `address` are mapped,
/// readable, contain a valid representation of `T`, and still belong to the object this callback
/// established. No concurrent write may race the read. Use this only for borrowed scalar/raw-pointer
/// fields, never an owning native object. It does not check mapping or extend any lifetime.
pub(crate) unsafe fn read<T: Copy>(address: usize) -> T {
    (address as *const T).read_unaligned()
}

/// Number of `stride`-byte entries between two vector pointers stored at `begin` / `end`.
///
/// # Safety
///
/// `begin` and `end` must be readable scalar globals (`usize`) for this callback, with no racing
/// writes, and `stride` must be nonzero. This computes a difference only: it does not bound the
/// count or validate either pointer or the pointed-to vector.
pub(crate) unsafe fn count(begin: usize, end: usize, stride: usize) -> usize {
    let (b, e): (usize, usize) = (read(begin), read(end));
    if e >= b {
        (e - b) / stride
    } else {
        0
    }
}

/// A pointer that can be a live game object: user-space, 8-aligned, not a small integer.
///
/// This is a cheap range/alignment heuristic, not proof of mapping, ownership, identity or
/// lifetime. Revalidate with the native class/state fields before any write.
pub(crate) fn plausible(p: usize) -> bool {
    p >= 0x10000 && p < 0x7fff_ffff_0000 && p % 8 == 0
}

/// A creature that is still part of the battle at this instant: plausible pointer, creature
/// class code, active, not dead.
///
/// # Safety
///
/// If `p` passes `plausible`, the caller must independently establish a current, readable native
/// creature whose fields remain valid for these game-thread reads. Implausible values return false
/// without dereferencing. This is not an identity/lifetime check for a cached address.
pub(crate) unsafe fn live_ship(p: usize) -> bool {
    plausible(p)
        && read::<u8>(p + OFF_CODE) & 0x2f == 0x2f
        && read::<i32>(p + OFF_ACTIVE) != 0
        && !matches!(read::<i32>(p + OFF_STATE), 6 | 7)
}

/// The current battle roster entries (creature pointers).
///
/// # Safety
///
/// Call on the game thread before native traversal. The returned `Vec` owns only the copied
/// `usize` values, not the creatures: every pointer is game-owned and may be freed or reused
/// after any native call. Never drop/free through these values; revalidate before writing.
pub(crate) unsafe fn roster() -> Vec<usize> {
    let (b, e): (usize, usize) = (read(ROSTER_BEGIN), read(ROSTER_END));
    if b < 0x10000 || e < b || (e - b) / LINK_STRIDE > SHIP_LIST_LIMIT {
        return Vec::new();
    }
    (0..(e - b) / LINK_STRIDE)
        .map(|i| read::<usize>(b + i * LINK_STRIDE + 8))
        .filter(|p| *p != 0)
        .collect()
}
