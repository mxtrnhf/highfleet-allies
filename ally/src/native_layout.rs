//! Steam 1.163 layout definitions shared with highfleet-rs.
//!
//! These types describe borrowed game memory only. Access fields with offsets or
//! raw field pointers: never read an owning string/container/object by value.
//! Keep native constructors, string assignment, and link registration in charge
//! of game-owned storage. See carrier-plane-loadouts.md and precombat-screen.md
//! in hf_notes/docs/findings for the remaining local layouts.

use std::mem::{offset_of, size_of};

pub use highfleet::general::{
    CVec, EscadraString, IObject, ISerializable, Link, Node, SentinelList, Tll,
};
pub use highfleet::v1_163::{
    AircraftLogic, Ammo, Body, Creature, Gui, Loadout, Munition, PartStats, Plane,
};

pub type PlaneNode = Tll<EscadraString, Plane>;

/// Native class code of a `Node` (`IObject::code`); the low byte carries the class bits.
pub const NODE_CODE: usize =
    offset_of!(Node, base) + offset_of!(ISerializable, base) + offset_of!(IObject, code);
/// Owner node of a part (`Node::owner` target).
pub const NODE_OWNER: usize = offset_of!(Node, owner) + offset_of!(Link, target);

// Fail at compile time if an upstream layout changes our 1.163 contract.
const _: () = {
    assert!(size_of::<Ammo>() == 0x188);
    assert!(size_of::<EscadraString>() == 0x20);
    assert!(size_of::<CVec<usize>>() == 0x18);
    assert!(size_of::<Link>() == 0x18);
    assert!(size_of::<Body>() == 0xa40);
    assert!(size_of::<Gui>() == 0xe70);
    assert!(size_of::<Plane>() == 0x20);
    assert!(size_of::<Loadout>() == 0x60);
    assert!(size_of::<Munition>() == 0x28);
    assert!(size_of::<AircraftLogic>() == 0x1b0);
    assert!(size_of::<Creature>() == 0x1a50);
    assert!(size_of::<PartStats>() == 0x150);
    assert!(offset_of!(Node, state) == 0x48);
    assert!(offset_of!(Node, master) + offset_of!(Link, target) == 0x88);
    assert!(NODE_OWNER == 0xb8);
    assert!(NODE_CODE == 0x28);
    assert!(offset_of!(Body, position_x) == 0x128);
    assert!(offset_of!(Body, position_y) == 0x12c);
    assert!(offset_of!(Body, health) == 0x2c8);
    assert!(offset_of!(Body, health_max) == 0x2cc);
    assert!(offset_of!(Body, logic) == 0x3c4);
    assert!(offset_of!(Body, oid) == 0x190);
    assert!(offset_of!(Body, logic_s1) == 0x3f8);
    assert!(offset_of!(Body, projectile_link_1) == 0x490);
    assert!(offset_of!(Body, projectile_link_2) == 0x4a8);
    assert!(offset_of!(Body, part_index) == 0x188);
    assert!(offset_of!(Body, weapon_ammo_amount) == 0x858);
    assert!(offset_of!(Body, weapon_ammo_loaded) == 0x860);
    assert!(offset_of!(Body, sprites) == 0x8c0);
    assert!(offset_of!(Body, opacity) == 0x8f8);
    assert!(offset_of!(Body, floor) == 0x9b8);
    assert!(offset_of!(Body, aircraft_logic) == 0xa20);
    assert!(offset_of!(Link, target) == 8);
    assert!(offset_of!(Ammo, item_name) == 8);
    assert!(offset_of!(Ammo, reticle) == 0);
    assert!(offset_of!(Ammo, logic) == 0x150);
    assert!(offset_of!(Ammo, caliber) == 0x154);
    assert!(offset_of!(Ammo, speed) == 0x158);
    assert!(offset_of!(Ammo, ap_drag) == 0x15c);
    assert!(offset_of!(Ammo, penetrative_power) == 0x164);
    assert!(offset_of!(Ammo, lifespan) == 0x16c);
    assert!(offset_of!(Body, velocity_x) == 0x130);
    assert!(offset_of!(Body, velocity_y) == 0x134);
    assert!(offset_of!(Body, weapon_ammo) == 0x850);
    assert!(offset_of!(Ammo, fire_delay) == 0x17c);
    assert!(offset_of!(Ammo, salvo_size) == 0x180);
    assert!(offset_of!(Munition, name) == 0);
    assert!(offset_of!(Munition, count) == 0x20);
    assert!(offset_of!(PlaneNode, left) == 0);
    assert!(offset_of!(PlaneNode, parent) == 8);
    assert!(offset_of!(PlaneNode, right) == 0x10);
    assert!(offset_of!(PlaneNode, is_sentinel) == 0x19);
    assert!(offset_of!(PlaneNode, key) == 0x20);
    assert!(offset_of!(PlaneNode, data) + offset_of!(Plane, loadouts) == 0x48);
    assert!(offset_of!(Plane, cached_launch_weight_sum) == 0);
    assert!(offset_of!(Plane, loadouts) == 0x08);
    assert!(offset_of!(Loadout, parts) == 0x40);
    assert!(offset_of!(Loadout, launch_loadout_weight) == 0x58);
    assert!(offset_of!(Loadout, has_gun37mm) == 0x5c);
    assert!(offset_of!(AircraftLogic, gun_reload_duration) == 0xa0);
    assert!(offset_of!(AircraftLogic, gun_reload_timer) == 0xa4);
    assert!(offset_of!(AircraftLogic, gun_rate) == 0xa8);
    assert!(offset_of!(AircraftLogic, gun_load) == 0xac);
    assert!(offset_of!(AircraftLogic, gun_max_load) == 0xb0);
    assert!(offset_of!(AircraftLogic, gun_reserve_ammo) == 0xb4);
    assert!(offset_of!(AircraftLogic, mode) == 0x118);
    assert!(offset_of!(AircraftLogic, bomb_ammo_indices) == 0x120);
    assert!(offset_of!(AircraftLogic, rocket_ammo_indices) == 0x130);
    assert!(offset_of!(AircraftLogic, missile_ammo_indices) == 0x140);
    assert!(offset_of!(AircraftLogic, target_link) == 0x150);
    assert!(offset_of!(AircraftLogic, state) == 0x190);
    assert!(offset_of!(AircraftLogic, gun_ammo_index) == 0x1a4);
    assert!(offset_of!(Creature, side) == 0xad4);
    assert!(offset_of!(Creature, target) == 0xaf8);
    assert!(offset_of!(Creature, retarget_timer) == 0xb28);
    assert!(offset_of!(Creature, mode) == 0xb48);
    assert!(offset_of!(Creature, trigger_gun_group_0) == 0xbbc);
    assert!(offset_of!(Creature, trigger_gun_group_1) == 0xbbd);
    assert!(offset_of!(Creature, trigger_bomb) == 0xbbe);
    assert!(offset_of!(Creature, trigger_flare) == 0xbc0);
    assert!(offset_of!(Creature, ai_flag) == 0xbd4);
    assert!(offset_of!(Creature, driver_state) == 0xbdc);
    assert!(offset_of!(Creature, waypoint_x) == 0xbf0);
    assert!(offset_of!(Creature, waypoint_y) == 0xbf4);
    assert!(offset_of!(Creature, order_role) == 0xd70);
    assert!(offset_of!(Creature, parts) == 0x1330);
    assert!(offset_of!(Creature, ammo_consumers) == 0x1360);
    assert!(offset_of!(Creature, gun_group_0) == 0x13a8);
    assert!(offset_of!(Creature, gun_group_1) == 0x13c0);
    assert!(offset_of!(Creature, anti_ship_launchers) == 0x13d8);
    assert!(offset_of!(Creature, flare_launchers) == 0x1408);
    assert!(offset_of!(Creature, bombs) == 0x1438);
    assert!(offset_of!(Creature, planes) == 0x14c8);
    assert!(offset_of!(Creature, mass) == 0x15b8);
    assert!(offset_of!(Creature, fuel) == 0x1638);
    assert!(offset_of!(Creature, fuel_capacity) == 0x1640);
    assert!(offset_of!(Creature, thrust_group_0) == 0x1664);
    assert!(offset_of!(Creature, weapon_reach_group_0) == 0x1838);
    assert!(offset_of!(Creature, weapon_reach_group_1) == 0x183c);
    assert!(offset_of!(Creature, bounds_center_x) == 0x1840);
    assert!(offset_of!(Creature, bounds_center_y) == 0x1844);
    assert!(offset_of!(Creature, bounding_radius) == 0x1850);
    assert!(offset_of!(PartStats, category) == 0xb0);
    assert!(offset_of!(PartStats, caliber) == 0x144);
    assert!(offset_of!(Gui, identifier) == 0xa40);
    assert!(offset_of!(Gui, gui_l1) + offset_of!(Link, target) == 0xbc0);
    assert!(offset_of!(Gui, gui_i1) == 0xc00);
};
