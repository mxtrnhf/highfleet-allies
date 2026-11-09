//! Mod entry points: version compatibility, config application, and the one patch session that
//! coordinates every subsystem installer.

use std::ffi::{c_char, CStr};
use std::sync::atomic::{AtomicBool, Ordering};

use patchy::{PatchSession, ProcessModule};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleA;

use crate::config::{parse_hex_color, Config};
use crate::native::IMAGE_BASE;
use crate::{
    ally_ammo, ally_hud, ally_panels, ally_reticles, battle, bomber, diag, enemy_waves,
    honor_share, intercept, lifecycle_log, missile_guard, plane_loadouts, precombat, ship_switch,
    sound,
};

static GAME_IS_STEAM_1_163: AtomicBool = AtomicBool::new(false);

/// Accepts the loader's supported-game version handshake.
///
/// # Safety
/// `version` must point to a readable NUL-terminated string for this call.
pub(crate) unsafe fn version(version: *const c_char) -> bool {
    let supported = CStr::from_ptr(version).to_bytes() == b"Steam 1.163";
    GAME_IS_STEAM_1_163.store(supported, Ordering::Relaxed);
    supported
}

/// Loads settings and installs the mod, returning false on incompatibility or a fatal install error.
///
/// # Safety
/// Called once by the Windows modloader during serialized initialization in the game process.
/// No game callbacks may run concurrently with native byte verification and permanent patching.
pub(crate) unsafe fn init() -> bool {
    let path = crate::config::PATH;
    let config: Config = crate::config::load();
    let supported = GAME_IS_STEAM_1_163.load(Ordering::Relaxed);
    let image_ok = GetModuleHandleA(std::ptr::null()) as usize == IMAGE_BASE;
    // debug_log is the runtime switch; set it before install so the legacy cargo feature cannot force it
    diag::set_debug(supported && image_ok && config.enabled && config.logging.debug_log);
    // the first line of this mod in every log: testers must be able to show which build they run
    log::info!("{}", diag::banner());
    diag::install();
    if !supported {
        log::error!("hf_ally: needs Steam 1.163, not starting");
        return false;
    }
    if !image_ok {
        log::error!("hf_ally: unexpected image base, not starting");
        return false;
    }
    if !config.enabled {
        log::info!("hf_ally: disabled in {path}");
        return true;
    }
    if config.logging.debug_log {
        log::info!(
            "hf_ally: debug_log on: megalog, watchdog and crash dump (large files, see ally.json)"
        );
    }
    precombat::configure_marker(&config.marker_text);
    ally_ammo::set_enabled(config.ally_ai_ammo);
    missile_guard::set_autofire(config.ally_zenith_autofire);
    let honor_share = honor_share::set_share(config.ally_honor_share);
    if honor_share != config.ally_honor_share {
        log::warn!(
            "hf_ally: ally_honor_share {} is outside 0..=1 or not finite; using {honor_share}",
            config.ally_honor_share
        );
    }
    log::info!(
        "hf_ally v{} config: {}",
        env!("CARGO_PKG_VERSION"),
        serde_json::to_string(&config).unwrap_or_default()
    );
    battle::configure(&config);

    match prepare(&config) {
        Ok(()) => {
            log::info!(
                "hf_ally v{}: hooks installed (spawn log, creature log {})",
                env!("CARGO_PKG_VERSION"),
                config.logging.log_creatures
            );
            true
        }
        Err(error) => {
            log::error!("hf_ally: not installed: {error}");
            false
        }
    }
}

unsafe fn prepare(config: &Config) -> Result<(), String> {
    let module = ProcessModule::main().map_err(|e| e.to_string())?;
    let lifecycle = lifecycle_log::verify_hooks(&module, config.logging.log_creatures)?;

    if sound::verify() {
        sound::configure(config.ally_sounds, true);
    }

    let mut session = PatchSession::new();
    lifecycle_log::install_hooks(&mut session, lifecycle, config.logging.log_creatures)?;
    // The pre-combat ALLY and plane-loadout buttons are part of the mod; install them whenever it is enabled.
    precombat::install(&mut session, config.logging.gui_inspect)?;
    ally_ammo::install_refund(&mut session, config.ally_ai_ammo)?;
    battle::install_prize_guard(&mut session, config.spare_prizes)?;
    let guard_missiles = config.ally_bombers || !config.ally_zenith_autofire;
    if guard_missiles {
        missile_guard::install(&mut session)?;
    }
    if config.ally_bombers {
        bomber::install(&mut session)?;
    }
    // the filter applies to every AI ship, the enemy's included
    if let Err(error) = intercept::install(&mut session, &config.intercept_ignore_oids) {
        log::warn!("hf_ally: {error}");
    }
    enemy_waves::install(&mut session, config.enemy_group_size);
    if config.plane_loadouts {
        match plane_loadouts::install(&mut session) {
            Ok(()) => {
                precombat::set_plane_buttons(true);
                plane_loadouts::install_aa(&mut session);
                plane_loadouts::install_no_rocket_draw(&mut session);
                plane_loadouts::install_test_stock(&mut session);
                plane_loadouts::install_aa_limits(
                    &mut session,
                    config.plane_aa_range,
                    config.plane_aa_angle,
                );
                plane_loadouts::AA_DELAY_S.store(
                    config.plane_aa_delay.clamp(0.0, 60.0).to_bits(),
                    Ordering::Relaxed,
                );
                plane_loadouts::AA_SPEED.store(config.plane_aa_speed.to_bits(), Ordering::Relaxed);
            }
            Err(error) => log::warn!("hf_ally: {error}"),
        }
    }
    plane_loadouts::ATTACK_DELAY_S.store(
        config.plane_attack_delay.clamp(0.0, 120.0).to_bits(),
        Ordering::Relaxed,
    );
    let ally_damage_color = match parse_hex_color(&config.ally_damage_color) {
        Some(color) => color,
        None => {
            log::warn!(
                "hf_ally: ally_damage_color {:?} is not HEX (want \"#RRGGBB\"); using #00E4FF",
                config.ally_damage_color
            );
            0x00e4ff
        }
    };
    ally_hud::install(&mut session, ally_damage_color);
    if config.ally_panels {
        ally_panels::install(&mut session);
    }
    let reticle_scale = config.ally_reticle_scale;
    if reticle_scale != 0.0 || config.debug_overlay.enable {
        // the reticle module owns the single per-frame battle HUD post-pass that both features draw from
        ally_reticles::install(
            &mut session,
            ally_damage_color,
            reticle_scale,
            &config.debug_overlay,
        );
    }
    plane_loadouts::EXTRA_GUNPOD.store(config.extra_gunpod, Ordering::Relaxed);
    plane_loadouts::K13_ON.store(config.plane_k13, Ordering::Relaxed);
    // Wingman deployment and battle-manager hooks; always installed when the mod is enabled.
    let battle = battle::verify_battle_hooks(&module)?;
    ship_switch::configure(config.ship_switching)?;
    battle::install_battle_hook(&mut session, battle)?;
    if honor_share::share() > 0.0 {
        honor_share::verify_native()?;
        // SAFETY: the 1.163 site bytes were just checked; this is mod initialization.
        unsafe { honor_share::install(&mut session)? };
        log::info!(
            "hf_ally: ally honor sharing at {:.0}%",
            honor_share::share() * 100.0
        );
    }
    session.install_permanently().map_err(|e| e.to_string())
}
