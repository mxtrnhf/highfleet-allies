//! Steam 1.163 layouts and field offsets shared with highfleet-rs.
//!
//! The types are borrowed live views of game memory. Read them through raw
//! pointers and field offsets, page-checked by the callers: never move, drop or
//! take ownership of a game-owned string, container or object.
//! `NODE_CODE`, `NODE_OWNER` and the `*_TARGET` offsets address a `Link`'s
//! target or an object's class code inside a borrowed object. The remaining raw
//! offsets in this mod cover fields upstream does not model yet.

use std::mem::{offset_of, size_of};

pub use highfleet::general::{IObject, ISerializable, Link, Node};
pub use highfleet::v1_163::{Body, Creature, PartStats};

/// Native class code of a `Node` (`IObject::code`); the low byte carries the class bits.
pub const NODE_CODE: usize =
    offset_of!(Node, base) + offset_of!(ISerializable, base) + offset_of!(IObject, code);
/// Active `Node` state word (`m_state`); non-zero while the object is in the level.
pub const NODE_STATE: usize = offset_of!(Node, state);
/// Owner node of a part (`Node::owner` target).
pub const NODE_OWNER: usize = offset_of!(Node, owner) + offset_of!(Link, target);
/// Current target of a `Creature` (`Creature::target` target).
pub const CREATURE_TARGET: usize = offset_of!(Creature, target) + offset_of!(Link, target);
/// Side float of a `Creature`; negative for the AI's enemies.
pub const CREATURE_SIDE: usize = offset_of!(Creature, side);
/// Raw AI control flag of a `Creature`; zero while the player flies the ship.
pub const CREATURE_AI_FLAG: usize = offset_of!(Creature, ai_flag);
/// Native mode integer of a `Creature`.
pub const CREATURE_MODE: usize = offset_of!(Creature, mode);
/// AI driver state of a `Creature`; 6 and 7 mean dead or gone.
pub const CREATURE_DRIVER_STATE: usize = offset_of!(Creature, driver_state);
/// Raw group-0 gun trigger byte.
pub const CREATURE_TRIGGER_GUN_GROUP_0: usize = offset_of!(Creature, trigger_gun_group_0);
/// Ship position used by the AI (bounding-box centre, not a mass centre).
pub const CREATURE_POS_X: usize = offset_of!(Creature, bounds_center_x);
pub const CREATURE_POS_Y: usize = offset_of!(Creature, bounds_center_y);
/// Ship bounding radius.
pub const CREATURE_RADIUS: usize = offset_of!(Creature, bounding_radius);
/// Reach of the group-0 guns, read together with the group-1 word after it by the AI range gate.
pub const CREATURE_WEAPON_REACH_0: usize = offset_of!(Creature, weapon_reach_group_0);
/// Creature `CVec<Link>` header offsets; `+ size_of::<usize>()` is its initialized-end word.
pub const CREATURE_FLARE_LAUNCHERS: usize = offset_of!(Creature, flare_launchers);
pub const CREATURE_GUN_GROUP_0: usize = offset_of!(Creature, gun_group_0);
pub const CREATURE_GUN_GROUP_1: usize = offset_of!(Creature, gun_group_1);
/// Definition-table index of a part `Body`, used by `GetStats`.
pub const BODY_PART_INDEX: usize = offset_of!(Body, part_index);
/// The plane part's `AircraftLogic` pointer.
pub const BODY_AIRCRAFT_LOGIC: usize = offset_of!(Body, aircraft_logic);
/// Part position and flight vector.
pub const BODY_POS_X: usize = offset_of!(Body, position_x);
pub const BODY_POS_Y: usize = offset_of!(Body, position_y);
pub const BODY_HEADING: usize = offset_of!(Body, angle);
/// Runtime projectile links: `_1` is the missile's target, `_2` its launching ship for guided launches.
pub const BODY_PROJECTILE_TARGET: usize =
    offset_of!(Body, projectile_link_1) + offset_of!(Link, target);
pub const BODY_PROJECTILE_OWNER: usize =
    offset_of!(Body, projectile_link_2) + offset_of!(Link, target);
/// Loaded rounds of a part, used for gun and launcher reload state.
pub const BODY_AMMO_LOADED: usize = offset_of!(Body, weapon_ammo_loaded);
/// Classifying part-stat field.
pub const STATS_CATEGORY: usize = offset_of!(PartStats, category);
/// One `Link` entry in a native `CVec<Link>` gun or launcher list.
pub const LINK_STRIDE: usize = size_of::<Link>();

// Fail at compile time if an upstream layout changes the 1.163 contract above.
const _: () = {
    assert!(size_of::<Link>() == 0x18);
    assert!(size_of::<Node>() == 0x128);
    assert!(size_of::<Body>() == 0xa40);
    assert!(size_of::<Creature>() == 0x1a50);
    assert!(size_of::<PartStats>() == 0x150);
    assert!(NODE_CODE == 0x28);
    assert!(NODE_STATE == 0x48);
    assert!(NODE_OWNER == 0xb8);
    assert!(CREATURE_TARGET == 0xb00);
    assert!(CREATURE_SIDE == 0xad4);
    assert!(CREATURE_AI_FLAG == 0xbd4);
    assert!(CREATURE_MODE == 0xb48);
    assert!(CREATURE_DRIVER_STATE == 0xbdc);
    assert!(CREATURE_TRIGGER_GUN_GROUP_0 == 0xbbc);
    assert!(offset_of!(Creature, trigger_gun_group_1) == 0xbbd);
    assert!(offset_of!(Creature, trigger_bomb) == 0xbbe);
    assert!(offset_of!(Creature, trigger_flare) == 0xbc0);
    assert!(offset_of!(Creature, waypoint_x) == 0xbf0);
    assert!(offset_of!(Creature, waypoint_y) == 0xbf4);
    assert!(CREATURE_POS_X == 0x1840);
    assert!(CREATURE_POS_Y == 0x1844);
    assert!(CREATURE_RADIUS == 0x1850);
    assert!(CREATURE_WEAPON_REACH_0 == 0x1838);
    assert!(offset_of!(Creature, weapon_reach_group_1) == 0x183c);
    assert!(CREATURE_FLARE_LAUNCHERS == 0x1408);
    assert!(CREATURE_GUN_GROUP_0 == 0x13a8);
    assert!(CREATURE_GUN_GROUP_1 == 0x13c0);
    assert!(BODY_PART_INDEX == 0x188);
    assert!(BODY_AIRCRAFT_LOGIC == 0xa20);
    assert!(BODY_POS_X == 0x128);
    assert!(BODY_POS_Y == 0x12c);
    assert!(BODY_HEADING == 0x138);
    assert!(BODY_PROJECTILE_TARGET == 0x498);
    assert!(BODY_PROJECTILE_OWNER == 0x4b0);
    assert!(BODY_AMMO_LOADED == 0x860);
    assert!(STATS_CATEGORY == 0xb0);
    assert!(LINK_STRIDE == 0x18);
};
