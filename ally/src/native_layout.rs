//! Steam 1.163 layout definitions shared with highfleet-rs.
//!
//! These types describe borrowed game memory only. Access fields with offsets or
//! raw field pointers: never read an owning string/container/object by value.
//! Keep native constructors, string assignment, and link registration in charge
//! of game-owned storage. See carrier-plane-loadouts.md and precombat-screen.md
//! in hf_notes/docs/findings for the remaining local layouts.

use std::mem::{offset_of, size_of};

pub use highfleet::general::{CVec, EscadraString, Link, Node, Tll};
pub use highfleet::v1_163::{Ammo, Body, Gui};

// Upstream does not yet describe these aircraft table payloads.
#[repr(C)]
pub struct Plane {
    unknown: [u8; 8],
    pub loadouts: CVec<*const Loadout>,
}

#[repr(C)]
pub struct Loadout {
    pub oid: EscadraString,
    icon: EscadraString,
    pub parts: CVec<Munition>,
    launch_weight: u32,
    has_gun37: u8,
    unknown: [u8; 3],
}

#[repr(C)]
pub struct Munition {
    pub name: EscadraString,
    pub count: i32,
    unknown: [u8; 4],
}

pub type PlaneNode = Tll<EscadraString, Plane>;

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
    assert!(offset_of!(Node, state) == 0x48);
    assert!(offset_of!(Body, position_x) == 0x128);
    assert!(offset_of!(Body, position_y) == 0x12c);
    assert!(offset_of!(Body, oid) == 0x190);
    assert!(offset_of!(Body, logic_s1) == 0x3f8);
    assert!(offset_of!(Body, aircraft_logic) == 0xa20);
    assert!(offset_of!(Link, target) == 8);
    assert!(offset_of!(Ammo, item_name) == 8);
    assert!(offset_of!(Ammo, reticle) == 0);
    assert!(offset_of!(Munition, name) == 0);
    assert!(offset_of!(Munition, count) == 0x20);
    assert!(offset_of!(PlaneNode, left) == 0);
    assert!(offset_of!(PlaneNode, parent) == 8);
    assert!(offset_of!(PlaneNode, right) == 0x10);
    assert!(offset_of!(PlaneNode, is_sentinel) == 0x19);
    assert!(offset_of!(PlaneNode, key) == 0x20);
    assert!(offset_of!(PlaneNode, data) + offset_of!(Plane, loadouts) == 0x48);
    assert!(offset_of!(Loadout, parts) == 0x40);
    assert!(offset_of!(Ammo, fire_delay) == 0x17c);
    assert!(offset_of!(Ammo, salvo_size) == 0x180);
    assert!(offset_of!(Gui, identifier) == 0xa40);
    assert!(offset_of!(Gui, gui_l1) + offset_of!(Link, target) == 0xbc0);
    assert!(offset_of!(Gui, gui_i1) == 0xc00);
};
