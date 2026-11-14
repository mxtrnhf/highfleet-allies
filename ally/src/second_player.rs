//! A second human player's ship in the small panel row.
//!
//! The game keeps the ships flown by human players in the profile's player table (`+0x50`, 0x18 bytes per
//! entry). Entry 1 holds the ship a second player flies, when there is one. That ship is not in the ally
//! roster, so the panel code asks this module for it and gives it the first small panel; the allies follow.
//! Nothing here knows who fills that entry: it only reads the game's own table.

use crate::native::{live_ship, plausible, read, GetProfileFn, GET_PROFILE, OFF_AI_FLAG, OFF_SIDE};

const PROFILE_PLAYERS: usize = 0x50;
const PROFILE_PLAYER_STRIDE: usize = 0x18;

/// The raw pointer in profile player slot 1 (0 when the slot is empty or the profile is not readable).
pub unsafe fn raw() -> usize {
    let get_profile: GetProfileFn = std::mem::transmute(GET_PROFILE);
    let profile = get_profile();
    if !plausible(profile) {
        return 0;
    }
    read::<usize>(profile + PROFILE_PLAYERS + PROFILE_PLAYER_STRIDE)
}

/// The live player-side ship in profile slot 1, if any.
pub unsafe fn ship() -> Option<usize> {
    let ship = raw();
    (plausible(ship) && live_ship(ship) && read::<f32>(ship + OFF_SIDE) > 0.0).then_some(ship)
}

/// A ship a human is flying (never a number-key switch target).
pub unsafe fn is_player_flown(ship: usize) -> bool {
    read::<u8>(ship + OFF_AI_FLAG) == 0
}
