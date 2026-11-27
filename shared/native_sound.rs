//! Native sound and localized radio playback on the game thread.

use crate::native::{read, GUI_ROOT};
use std::sync::atomic::{AtomicU32, Ordering};

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

const UNIT_VOLUME_BITS: u32 = 1.0f32.to_bits();
/// The game retains the status pointer (sound object +0x88) and uses its float as a
/// volume multiplier each frame. The slots must outlive the native call.
static SFX_STATUS: [AtomicU32; 16] = [const { AtomicU32::new(UNIT_VOLUME_BITS) }; 16];
static SFX_NEXT: AtomicU32 = AtomicU32::new(0);

/// Stock `Media/Snd/button_01.wav`, named `button_01` in `sound.res`. The game itself plays it as a UI click in
/// `Map_BuildMissileMarkers` (`0x1401c4ee0`), with kind 0xb and base volume 30.0.
pub const SND_BUTTON: &[u8] = b"button_01\0";
pub const BUTTON_VOLUME: f32 = 30.0;

/// Checks that the game function this module calls is the expected build, then logs why it is off otherwise.
///
/// # Safety
/// Reads the fixed `PLAY_SFX` code address; callers must be on the game process after the module is loaded.
pub unsafe fn verify() -> bool {
    let found = std::slice::from_raw_parts(PLAY_SFX as *const u8, PLAY_SFX_PROLOGUE.len());
    if found != PLAY_SFX_PROLOGUE {
        log::warn!("playSFXSound_ bytes differ ({found:02x?}), mod sounds off");
        return false;
    }

    true
}

/// Plays one sound by its `sound.res` name (NUL-terminated); `what` is for the log.
///
/// # Safety
/// Call on the game thread after [`verify`] accepts the supported image, while its GUI root
/// remains live. `name` must contain one NUL-terminated resource name.
pub unsafe fn play_sfx(name: &[u8], volume: f32, what: &str) {
    let result = play_sfx_silent(name, volume, 1.0);
    log::info!(
        "sound ({what}) {} -> {result}",
        String::from_utf8_lossy(&name[..name.len() - 1])
    );
}

/// Plays one sound without a log line. Used for the restrained shelling report.
///
/// # Safety
/// Call on the game thread after [`verify`] accepts the supported image, with a live GUI
/// root and a NUL-terminated resource name.
pub unsafe fn play_sfx_silent(name: &[u8], volume: f32, pitch: f32) -> f32 {
    let node: usize = read(GUI_ROOT);
    if node == 0 {
        return -1.0;
    }
    let slot_index = SFX_NEXT.fetch_add(1, Ordering::Relaxed) as usize % SFX_STATUS.len();
    let slot = &SFX_STATUS[slot_index];
    slot.store(UNIT_VOLUME_BITS, Ordering::Relaxed);
    let play: PlaySfxFn = std::mem::transmute(PLAY_SFX);
    play(
        SFX_KIND_STOCK,
        name.as_ptr(),
        node,
        volume,
        pitch,
        slot.as_ptr() as *mut f32,
        0,
        0,
        0,
        0,
        0,
        0.0,
        1,
    )
}

/// Appends `_eng` to a stock radio name in English, matching `String_AppendEngSuffix` and the
/// `sound.res` entries that point at `Media/Snd/RADIO_ENG`. Returns a NUL-terminated name.
pub unsafe fn localized_radio_name(name: &[u8]) -> Vec<u8> {
    let base = name.strip_suffix(&[0]).unwrap_or(name);
    let mut localized = Vec::with_capacity(base.len() + ENG_SUFFIX.len() + 1);
    localized.extend_from_slice(base);
    if read::<i32>(CORE_LANGUAGE) == LANGUAGE_ENGLISH {
        localized.extend_from_slice(ENG_SUFFIX);
    }
    localized.push(0);
    localized
}
