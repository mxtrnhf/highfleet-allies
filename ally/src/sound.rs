//! Ally radio and pre-combat button sounds.

pub(crate) use crate::native_sound::verify;
use crate::native_sound::{localized_radio_name, play_sfx, BUTTON_VOLUME, SND_BUTTON};
use crate::time::pick;
use std::sync::atomic::{AtomicBool, Ordering};

/// Ally radio lines: the retreat key, an ally joining the fight, an ally destroyed.
static RADIO_SOUNDS: AtomicBool = AtomicBool::new(false);
/// Pre-combat ALLY and plane-loadout button clicks (independent of `ally_sounds`).
static BUTTON_SOUNDS: AtomicBool = AtomicBool::new(false);

const SND_DANGER: &[u8] = b"mission_danger_01\0";
const SND_LOST: [&[u8]; 2] = [b"radio_stall_01\0", b"radio_lost_energy_01\0"];
const SND_RETREATED: [&[u8]; 3] = [
    b"mission_end_01\0",
    b"mission_end_02\0",
    b"mission_end_03\0",
];
const SND_JOIN: [&[u8]; 3] = [
    b"mission_start_01\0",
    b"mission_start_02\0",
    b"mission_start_03\0",
];

/// Applies the verified runtime switches: ally radio lines follow `ally_sounds`; button clicks are
/// part of the mod's pre-combat UI and follow `buttons`.
pub(crate) fn configure(radio: bool, buttons: bool) {
    RADIO_SOUNDS.store(radio, Ordering::Relaxed);
    BUTTON_SOUNDS.store(buttons, Ordering::Relaxed);
}

/// Plays a radio sound by its `sound.res` name, in the player's language.
unsafe fn play_radio(name: &[u8], what: &str) {
    if !RADIO_SOUNDS.load(Ordering::Relaxed) {
        return;
    }
    let localized = localized_radio_name(name);
    play_sfx(&localized, 60.0, what);
}

/// Plays the stock UI click for one of our pre-combat buttons.
pub(crate) unsafe fn button(what: &str) {
    if BUTTON_SOUNDS.load(Ordering::Relaxed) {
        play_sfx(SND_BUTTON, BUTTON_VOLUME, what);
    }
}

/// Plays the danger line (the fuel retreat and the retreat key).
pub(crate) unsafe fn danger(what: &str) {
    play_radio(SND_DANGER, what);
}

/// Plays one of the mission-start lines when an ally joins the fight.
pub(crate) unsafe fn joined() {
    play_radio(SND_JOIN[pick(SND_JOIN.len())], "ally joins");
}

/// Plays one of the lost-energy/stall lines when an ally is destroyed.
pub(crate) unsafe fn lost() {
    play_radio(SND_LOST[pick(SND_LOST.len())], "ally destroyed");
}

/// Plays one of the mission-end lines when an ally retreats alive.
pub(crate) unsafe fn retreated() {
    play_radio(SND_RETREATED[pick(SND_RETREATED.len())], "ally retreated");
}
