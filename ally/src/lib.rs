//! HighFleet Steam 1.163 ally mod: native DLL exports and subsystem composition.
//! Bootstrap owns installation; runtime state stays with its battle, GUI, ammo and diagnostic owners.
//! Config: `Modloader/config/ally.json`. Native evidence: `hf_notes/docs/findings/battle-ally.md`.

highfleet_mod_api::export_logger!();
patchy_host::export_patchy!();
hf_mod_support::export_gui_host!();

use hf_mod_support::{
    aircraft_catalog, gui, gui_hooks, memory, native, native_layout, native_sound, panel_image,
    patchy, time,
};

/// A line for the debug mode's megalog (a no-op, and the arguments are not even formatted, otherwise).
macro_rules! crumb {
    ($($arg:tt)*) => {
        if $crate::diag::debug() {
            $crate::diag::crumb(&format!($($arg)*));
        }
    };
}

mod aim_math;
mod ally_ammo;
mod ally_hud;
mod ally_panels;
mod ally_reticles;
mod battle;
mod battle_diagnostics;
mod bomber;
mod bomber_math;
mod bootstrap;
mod config;
mod debug_overlay;
mod debug_state;
mod diag;
mod enemy_waves;
mod honor_share;
mod intercept;
mod lifecycle_log;
mod missile_guard;
mod plane_loadouts;
mod precombat;
// Shows the ship a second human player flies (profile slot 1) in the first small panel.
// Self-contained: it only reads the game's player table; it knows nothing about the mod that fills it.
mod second_player;
mod ship_switch;
mod sound;
mod template_health;

#[no_mangle]
unsafe extern "C" fn version(version: *const std::ffi::c_char) -> bool {
    bootstrap::version(version)
}

#[no_mangle]
unsafe extern "C" fn init() -> bool {
    bootstrap::init()
}
