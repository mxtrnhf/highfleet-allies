//! Campaign shelling and tactical barrage sounds.

use crate::native_sound::{localized_radio_name, play_sfx, play_sfx_silent};
use crate::time::pick;
use std::sync::atomic::{AtomicBool, Ordering};

/// Strategic shelling barrage, separate from radio chatter so it can be silenced alone.
static SHELLING_SOUNDS: AtomicBool = AtomicBool::new(false);

/// The four stock `Media/Snd/shell_out_big_far_*` samples.
const SND_SHELL: [&[u8]; 4] = [
    b"shell_out_big_far_01\0",
    b"shell_out_big_far_02\0",
    b"shell_out_big_far_03\0",
    b"shell_out_big_far_04\0",
];
const SND_SHELL_RUMBLE: [&[u8]; 4] = [
    b"explosion_big_far_01\0",
    b"explosion_big_far_02\0",
    b"explosion_big_far_03\0",
    b"explosion_big_far_04\0",
];
const SND_SHELL_WINDUP: &[u8] = b"gunfire_servo\0";
const SND_SHELL_RADIO_FIRE: [&[u8]; 4] = [
    b"radio_go1_01\0",
    b"radio_go2_01\0",
    b"radio_go3_01\0",
    b"radio_go4_01\0",
];
const SND_INTERCOM_OUT: &[u8] = b"intercom_out\0";
const SHELL_VOLUME: f32 = 56.0;
const SHELL_WINDUP_VOLUME: f32 = 52.0;
const SND_RAID_WIND: &[u8] = b"wind_02\0";
const RAID_WIND_VOLUME: f32 = 38.0;
const SND_RAID_WHOOSH: [&[u8]; 3] = [
    b"whoosh_fast_01\0",
    b"whoosh_fast_02\0",
    b"whoosh_fast_03\0",
];
const SND_RAID_DISTANT: [&[u8]; 4] = [
    b"explosion_small_far_01\0",
    b"explosion_small_far_02\0",
    b"explosion_small_far_03\0",
    b"explosion_small_far_04\0",
];
const SND_RAID_CRACK: [&[u8]; 4] = [
    b"explosion_med_01\0",
    b"explosion_med_02\0",
    b"explosion_med_03\0",
    b"explosion_med_04\0",
];
const SND_RAID_METAL: [&[u8]; 2] = [b"metall_hit_01\0", b"metall_hit_02\0"];
// These stock WAVs contain flat-topped transients at the start. Use the clipped
// source rather than clipping the master mix or stacking another sound on every hit.
const SND_RAID_CLIPPED: [&[u8]; 2] = [b"explosion_big_01\0", b"explosion_big_04\0"];

/// Enables or disables the shelling barrage report.
pub(crate) fn configure_shelling(enabled: bool) {
    SHELLING_SOUNDS.store(enabled, Ordering::Relaxed);
}

/// Let both concurrently played cues finish before the first barrel fires.
pub(crate) fn shelling_prelude_ms() -> u64 {
    if SHELLING_SOUNDS.load(Ordering::Relaxed) {
        // Stock servo is 4.484 s at unity pitch (4.671 s at our 0.96 pitch).
        4_750
    } else {
        0
    }
}

/// Plays a radio sound by its `sound.res` name, in the player's language.
unsafe fn play_radio(name: &[u8], what: &str) {
    if !SHELLING_SOUNDS.load(Ordering::Relaxed) {
        return;
    }
    let localized = localized_radio_name(name);
    play_sfx(&localized, 60.0, what);
}

/// Play heavy turret servo wind-up as cannons slew into firing elevation.
pub(crate) unsafe fn shelling_windup() {
    if SHELLING_SOUNDS.load(Ordering::Relaxed) {
        let result = play_sfx_silent(SND_SHELL_WINDUP, SHELL_WINDUP_VOLUME, 0.96);
        log::info!("CCU sound (shelling wind-up) gunfire_servo -> {result}");
    }
}

/// Play one randomly selected distant shot for one barrel in the opening salvo.
pub(crate) unsafe fn shelling_opening_shot(weight: u32) {
    if SHELLING_SOUNDS.load(Ordering::Relaxed) {
        let volume = SHELL_VOLUME + 12.0 * (weight.min(48) as f32 / 48.0);
        play_sfx(SND_SHELL[pick(SND_SHELL.len())], volume, "shelling opening");
    }
}

/// Localized crew radio confirmation when starting a bombardment.
pub(crate) unsafe fn shelling_radio_fire() {
    play_radio(
        SND_SHELL_RADIO_FIRE[pick(SND_SHELL_RADIO_FIRE.len())],
        "shelling fire",
    );
}

/// Short intercom click-off when aborting a bombardment.
pub(crate) unsafe fn shelling_radio_abort() {
    if SHELLING_SOUNDS.load(Ordering::Relaxed) {
        let _ = play_sfx_silent(SND_INTERCOM_OUT, 42.0, 1.0);
    }
}

/// One shot of a follow-up burst. The stock 01 WAV has ~5,100 clipped samples;
/// preserve that crack at the source instead of distorting the entire game mix.
pub(crate) unsafe fn shelling_burst_shot(weight: u32) {
    if SHELLING_SOUNDS.load(Ordering::Relaxed) {
        let pitch = 0.90 + pick(15) as f32 * 0.01;
        let volume = 80.0 + 12.0 * (weight.min(48) as f32 / 48.0);
        let _ = play_sfx_silent(SND_SHELL[0], volume, pitch);
    }
}

/// Start a single stock wind cue before any tactical rounds appear.
pub(crate) unsafe fn shelling_wind() {
    if SHELLING_SOUNDS.load(Ordering::Relaxed) {
        let _ = play_sfx_silent(SND_RAID_WIND, RAID_WIND_VOLUME, 1.0);
    }
}

/// Subtle brief incoming shell whoosh played on select close rounds right before impact.
pub(crate) unsafe fn shelling_whoosh(cue: u32) {
    if !SHELLING_SOUNDS.load(Ordering::Relaxed) {
        return;
    }
    let variant = ((cue >> 4) as usize) % SND_RAID_WHOOSH.len();
    let strength = ((cue >> 12) & 0xff) as f32 / 255.0;
    let pitch = 0.84 + ((cue >> 20) & 0xff) as f32 * (0.20 / 255.0);
    let _ = play_sfx_silent(SND_RAID_WHOOSH[variant], 36.0 + 12.0 * strength, pitch);
}

/// Vary loudness, pitch and the odds of a hard-clipped onset without changing the master mix.
pub(crate) unsafe fn shelling_impact(ground: bool, intensity: f32, cue: u32) {
    if !SHELLING_SOUNDS.load(Ordering::Relaxed) {
        return;
    }
    let variant = (cue >> 8) as usize;
    let strength = ((cue >> 16) & 0xff) as f32 / 255.0;
    let pitch = 0.86 + ((cue >> 24) & 0xff) as f32 * (0.32 / 255.0);
    let (sample, gain) = match (ground, cue & 15) {
        (true, 0) => return,
        (true, 1..=8) | (false, 0..=7) => {
            (SND_RAID_CLIPPED[variant % SND_RAID_CLIPPED.len()], 92.0)
        }
        (false, 8..=9) => (SND_RAID_METAL[variant % SND_RAID_METAL.len()], 74.0),
        (false, 10..=13) | (true, 9..=12) => (SND_RAID_CRACK[variant % SND_RAID_CRACK.len()], 80.0),
        _ => (SND_RAID_DISTANT[variant % SND_RAID_DISTANT.len()], 54.0),
    };
    let _ = play_sfx_silent(sample, gain + 14.0 * strength + 11.0 * intensity, pitch);
    // Layer a restrained low-end rolling echo on heavier hits so salvos rumble across the basin.
    if ((cue >> 5) & 3) == 0 && gain >= 80.0 {
        let rumble_pitch = 0.78 + ((cue >> 19) & 0x3f) as f32 * (0.12 / 63.0);
        let _ = play_sfx_silent(
            SND_SHELL_RUMBLE[variant % SND_SHELL_RUMBLE.len()],
            44.0 + 10.0 * intensity,
            rumble_pitch,
        );
    }
}
