//! Sound playback and localization for the mod's radio lines and pre-combat button clicks.
//!
//! `playSFXSound_` (0x1402e30e0) is what the stock battle radio (`PlayRadioSounds_`, 0x1402d5910) ends in; the call
//! below repeats its last call exactly: kind 0xb, the sound resource name from `Media/Snd/sound.res`, the global GUI
//! node, a base volume, then the stack arguments 1.0, a status float it writes into, and zeros. The stock radio also
//! appends `_eng` to its names when the game is in English, so `play_radio` does the same (v0.14.26).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::{pick, read, GUI_ROOT};

const PLAY_SFX: usize = 0x1_402e_30e0;
const PLAY_SFX_PROLOGUE: [u8; 15] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x48, 0x89, 0x68, 0x10, 0x48, 0x89, 0x70, 0x18,
];
/// Kind 0xb: the stock radio and the ClickFunc UI sounds all play through it.
const SFX_KIND_STOCK: i32 = 0xb;
type PlaySfxFn = unsafe extern "C" fn(
    i32,
    *const u8,
    usize,
    f32,
    f32,
    *mut f32,
    u64,
    u8,
    i32,
    u8,
    u8,
    f32,
    u8,
) -> f32;
/// `coreLanguage`: 1 = Russian, 2 = English. The options language button writes it live (`ClickFunc`
/// 0x140131af8), and the stock radio appends `_eng` exactly when it is 2 (`String_AppendEngSuffix`
/// 0x1401b20f0). Read it per line so a live language change applies without a restart.
const CORE_LANGUAGE: usize = 0x1_403c_0e1c;
const LANGUAGE_ENGLISH: i32 = 2;
/// `sound.res` maps the plain names to `Media/Snd/RADIO` (Russian) and the `_eng` names to
/// `Media/Snd/RADIO_ENG` (English); every line the mod plays has an `_eng` variant.
const ENG_SUFFIX: &[u8] = b"_eng";
/// Ally radio lines: the retreat key, an ally joining the fight, an ally destroyed.
static RADIO_SOUNDS: AtomicBool = AtomicBool::new(false);
/// Pre-combat ALLY and plane-loadout button clicks (independent of `ally_sounds`).
static BUTTON_SOUNDS: AtomicBool = AtomicBool::new(false);
/// The game keeps the pointer it is given (sound object +0x88) and, every frame, multiplies the sound's volume by
/// the float behind it (the stock radio stores the call's result, 1.0, there): 1.0f here, or the sound is silent.
static SFX_STATUS: [AtomicU32; 8] = [const { AtomicU32::new(0x3f80_0000) }; 8];
static SFX_NEXT: AtomicU32 = AtomicU32::new(0);

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
/// Stock `Media/Snd/button_01.wav`, named `button_01` in `sound.res`. The game itself plays it as a UI click in
/// `Map_BuildMissileMarkers` (`0x1401c4ee0`), with kind 0xb and base volume 30.0.
const SND_BUTTON: &[u8] = b"button_01\0";
const BUTTON_VOLUME: f32 = 30.0;

/// Checks that the game function this module calls is the expected build, then logs why it is off otherwise.
///
/// # Safety
/// Reads the fixed `PLAY_SFX` code address; callers must be on the game process after the module is loaded.
pub(crate) unsafe fn verify() -> bool {
    let found = std::slice::from_raw_parts(PLAY_SFX as *const u8, PLAY_SFX_PROLOGUE.len());
    if found == PLAY_SFX_PROLOGUE {
        true
    } else {
        log::warn!(
            "hf_ally: playSFXSound_ bytes differ ({found:02x?}), ally and button sounds off"
        );
        false
    }
}

/// Applies the verified runtime switches: ally radio lines follow `ally_sounds`; button clicks are
/// part of the mod's pre-combat UI and follow `buttons`.
pub(crate) fn configure(radio: bool, buttons: bool) {
    RADIO_SOUNDS.store(radio, Ordering::Relaxed);
    BUTTON_SOUNDS.store(buttons, Ordering::Relaxed);
}

/// Plays one sound by its `sound.res` name (NUL-terminated); `what` is for the log.
unsafe fn play_sfx(name: &[u8], volume: f32, what: &str) {
    let node: usize = read(GUI_ROOT);
    if node == 0 {
        log::info!("ALLY sound ({what}) skipped: no global node yet");
        return;
    }
    let slot = &SFX_STATUS[SFX_NEXT.fetch_add(1, Ordering::Relaxed) as usize % SFX_STATUS.len()];
    slot.store(0x3f80_0000, Ordering::Relaxed);
    let play: PlaySfxFn = std::mem::transmute(PLAY_SFX);
    let result = play(
        SFX_KIND_STOCK,
        name.as_ptr(),
        node,
        volume,
        1.0,
        slot.as_ptr() as *mut f32,
        0,
        0,
        0,
        0,
        0,
        0.0,
        1,
    );
    log::info!(
        "ALLY sound ({what}) {} -> {result}",
        String::from_utf8_lossy(&name[..name.len() - 1])
    );
}

/// Appends `_eng` to a stock radio name in English, matching `String_AppendEngSuffix` and the
/// `sound.res` entries that point at `Media/Snd/RADIO_ENG`. Returns a NUL-terminated name.
unsafe fn localized_radio_name(name: &[u8]) -> Vec<u8> {
    let base = name.strip_suffix(&[0]).unwrap_or(name);
    let mut localized = Vec::with_capacity(base.len() + ENG_SUFFIX.len() + 1);
    localized.extend_from_slice(base);
    if read::<i32>(CORE_LANGUAGE) == LANGUAGE_ENGLISH {
        localized.extend_from_slice(ENG_SUFFIX);
    }
    localized.push(0);
    localized
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
