use std::sync::atomic::{AtomicBool, Ordering};

use crate::native_sound::{play_sfx, BUTTON_VOLUME, SND_BUTTON};

static ENABLED: AtomicBool = AtomicBool::new(false);

pub(crate) fn configure(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

/// Play a stock GUI click after verifying native sound playback at startup.
pub(crate) unsafe fn button(what: &str) {
    if ENABLED.load(Ordering::Relaxed) {
        play_sfx(SND_BUTTON, BUTTON_VOLUME, what);
    }
}
