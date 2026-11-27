//! Standalone Shipworks rosters and Advanced Custom Encounters.

use serde::{Deserialize, Serialize};
use std::ffi::c_char;

highfleet_mod_api::export_logger!();
patchy_host::export_patchy!();
hf_mod_support::export_gui_host!();

use hf_mod_support::{
    aircraft_catalog, gui, gui_hooks, memory, native, native_layout, native_sound, panel_image,
    patchy, ship_health, standalone, time,
};

mod shipworks;
mod sound;

const CONFIG_PATH: &str = "Modloader/config/aces.json";

#[derive(Deserialize, Serialize)]
#[serde(default)]
struct Config {
    enable: bool,
    crash_button: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enable: true,
            crash_button: false,
        }
    }
}

#[no_mangle]
unsafe extern "C" fn version(version: *const c_char) -> bool {
    standalone::version(version)
}

#[no_mangle]
unsafe extern "C" fn init() -> bool {
    match install() {
        Ok(()) => true,
        Err(error) => {
            log::error!("ACES: {error}");
            false
        }
    }
}

unsafe fn install() -> Result<(), String> {
    standalone::check_game()?;
    let config: Config = standalone::config(CONFIG_PATH)?;
    if !config.enable {
        log::info!("ACES: disabled in {CONFIG_PATH}");
        return Ok(());
    }

    sound::configure(native_sound::verify());
    let mut session = patchy::PatchSession::new().map_err(|e| e.to_string())?;
    shipworks::install(&mut session, config.crash_button)?;
    // SAFETY: Modloader serializes init and keeps this DLL alive for the process.
    gui_hooks::commit(
        session,
        gui_hooks::ACES,
        gui_hooks::Callbacks {
            tick,
            click,
            swap_mask: None,
        },
    )?;
    log::info!("ACES v{}: hooks installed", env!("CARGO_PKG_VERSION"));
    Ok(())
}

unsafe extern "C" fn tick() {
    shipworks::tick();
}

unsafe extern "C" fn click(_: usize, element: usize, button: usize, _: usize) -> bool {
    let name = gui::cstr(element + gui::GUI_NAME);
    shipworks::on_click(&name, button)
}
