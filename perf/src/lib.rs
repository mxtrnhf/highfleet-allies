//! Experimental vanilla engine work elimination; independent of ally/AI/QoL.

mod ai_spans;
mod font_color;
mod frame_skip;
mod hud_hooks;
mod hud_query;
mod kaz_scan;
mod profile;
mod profile_memory;
mod spatial;

use std::{
    ffi::{c_char, CStr},
    sync::atomic::{AtomicBool, Ordering},
};

use patchy::{PatchSession, Trampoline};
use serde::{Deserialize, Serialize};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleA;

highfleet_mod_api::export_logger!();

const IMAGE_BASE: usize = 0x1_4000_0000;
const FONT_SET_COLOR: usize = IMAGE_BASE + 0x16f120;
const SPRITE_SET_COLOR: usize = IMAGE_BASE + 0x172ae0;
const CONFIG_PATH: &str = "Modloader/config/hf_perf.json";
// Full function bodies, not just prologues. Fail closed if another mod changes
// either the replaced loop or the trivial callee whose behavior we batch.
const FONT_BYTES: [u8; 79] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20, 0x8b,
    0xf2, 0x89, 0x91, 0x20, 0x10, 0x00, 0x00, 0x48, 0x8d, 0x59, 0x08, 0xbf, 0x00, 0x01, 0x00, 0x00,
    0x48, 0x8b, 0x0b, 0x48, 0x85, 0xc9, 0x74, 0x0d, 0x41, 0xb8, 0xff, 0xff, 0xff, 0xff, 0x8b, 0xd6,
    0xe8, 0x8b, 0x39, 0x00, 0x00, 0x48, 0x83, 0xc3, 0x08, 0x48, 0x83, 0xef, 0x01, 0x75, 0xe1, 0x48,
    0x8b, 0x5c, 0x24, 0x30, 0x48, 0x8b, 0x74, 0x24, 0x38, 0x48, 0x83, 0xc4, 0x20, 0x5f, 0xc3,
];
const SPRITE_BYTES: [u8; 38] = [
    0x41, 0x83, 0xf8, 0xff, 0x74, 0x0d, 0x49, 0x63, 0xc0, 0x4c, 0x6b, 0xc0, 0x58, 0x41, 0x89, 0x54,
    0x08, 0x18, 0xc3, 0x89, 0x91, 0x20, 0x01, 0x00, 0x00, 0x89, 0x91, 0xc8, 0x00, 0x00, 0x00, 0x89,
    0x51, 0x70, 0x89, 0x51, 0x18, 0xc3,
];

static SUPPORTED: AtomicBool = AtomicBool::new(false);

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    enabled: bool,
    hud_spatial_queries: bool,
    verify_queries: bool,
    kaz_fast_scan: bool,
    frame_skip: bool,
    profiling: bool,
    /// Native AI attribution spans (`ai_driver_*` / `ai_scan_*` CSV columns).
    /// Research switch: set false to measure the span hooks' own overhead.
    ai_spans: bool,
    profile_output_dir: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            hud_spatial_queries: true,
            verify_queries: true,
            kaz_fast_scan: true,
            frame_skip: false,
            profiling: false,
            ai_spans: true,
            profile_output_dir: String::new(),
        }
    }
}

fn read_config() -> Result<Config, String> {
    match std::fs::read_to_string(CONFIG_PATH) {
        Ok(text) => serde_json::from_str(&text).map_err(|error| error.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Experimental patch is opt-in, including on first installation.
            let config = Config::default();
            let text = serde_json::to_string_pretty(&config).map_err(|error| error.to_string())?;
            std::fs::write(CONFIG_PATH, text).map_err(|error| error.to_string())?;
            Ok(config)
        }
        Err(error) => Err(error.to_string()),
    }
}

#[no_mangle]
unsafe extern "C" fn version(version: *const c_char) -> bool {
    if version.is_null() {
        return false;
    }
    // SAFETY: Modloader v4 supplies a live NUL-terminated version string during
    // discovery; it stays borrowed for this comparison only.
    let supported = unsafe { CStr::from_ptr(version) }.to_bytes() == b"Steam 1.163";
    SUPPORTED.store(supported, Ordering::Relaxed);
    supported
}

#[no_mangle]
unsafe extern "C" fn init() -> bool {
    log::info!(
        "hf_perf v{}: frame/task profiling and optional engine optimizations",
        env!("CARGO_PKG_VERSION")
    );
    if !SUPPORTED.load(Ordering::Relaxed) {
        log::error!("hf_perf: needs Steam 1.163; no patches installed");
        return false;
    }
    // SAFETY: Null asks Windows for the currently loaded main executable.
    if unsafe { GetModuleHandleA(std::ptr::null()) } as usize != IMAGE_BASE {
        log::error!("hf_perf: unexpected executable base; no patches installed");
        return false;
    }
    let config = match read_config() {
        Ok(config) => config,
        Err(error) => {
            log::error!("hf_perf: invalid/unreadable {CONFIG_PATH}: {error}; no patches installed");
            return false;
        }
    };
    if !config.enabled && !config.profiling {
        log::info!("hf_perf: disabled in {CONFIG_PATH}; native engine unchanged");
        return true;
    }
    // SAFETY: Follow the existing mods' startup-only installation contract:
    // install from loader initialization, never inject into an executing scene.
    // Supported image/base are checked; quiescent startup is still required.
    hud_query::configure(config.verify_queries);
    match unsafe { install(&config) } {
        Ok(()) => {
            log::info!("hf_perf: optimizations={}, HUD spatial queries={}, KAZ fast scan={}, frame skip={}, profiling={}; in-game FPS gain unmeasured", config.enabled, config.enabled && config.hud_spatial_queries, config.enabled && config.kaz_fast_scan, config.enabled && config.frame_skip, config.profiling);
            true
        }
        Err(error) => {
            log::error!("hf_perf: not installed: {error}");
            false
        }
    }
}

/// Install only at startup with the supported executable mapped and no caller
/// executing the target. No part of this operation establishes heap validity.
unsafe fn install(config: &Config) -> Result<(), String> {
    if config.enabled {
        for (address, expected) in [
            (FONT_SET_COLOR, FONT_BYTES.as_slice()),
            (SPRITE_SET_COLOR, SPRITE_BYTES.as_slice()),
        ] {
            // SAFETY: These bounded code ranges are mapped read-only executable
            // sections of the supported main module, not arbitrary game pointers.
            let found = unsafe { std::slice::from_raw_parts(address as *const u8, expected.len()) };
            if found != expected {
                return Err(format!(
                    "native bytes differ at {address:#x}; another mod or unsupported build"
                ));
            }
        }
        if config.hud_spatial_queries {
            // SAFETY: Same supported mapped image/startup contract as font guards.
            unsafe { hud_hooks::validate()? };
        }
        if config.kaz_fast_scan {
            // SAFETY: Same fixed-image startup contract; KAZ validates its full
            // target loop and every replaced native helper before installation.
            unsafe { kaz_scan::validate()? };
        }
        if config.frame_skip {
            // SAFETY: Same fixed-image startup contract; only the video-task
            // entry prologue is read here.
            unsafe { frame_skip::validate()? };
        }
    }
    let mut session = PatchSession::new();
    if config.profiling {
        // SAFETY: same startup/version/base contract; prepare validates stolen
        // instructions before creating the asynchronous output session.
        unsafe { profile::prepare(&mut session, &config.profile_output_dir, config)? };
        if config.ai_spans {
            // Validate the entire optional group before preparing any of its
            // patches. A conflict must not discard the core frame/task hook.
            // SAFETY: same startup-only, fixed-image contract as core profiling.
            match unsafe { ai_spans::validate() } {
                Ok(()) => unsafe { ai_spans::prepare(&mut session)? },
                Err(error) => log::warn!(
                    "hf_perf: optional AI spans unavailable: {error}; core frame/task profiling continues; ai_driver/ai_scan columns will be blank"
                ),
            }
        }
    }
    if config.enabled {
        let mut trampoline = Trampoline::new();
        trampoline.absolute_jump(profiled_font_color as *const () as usize);
        // SAFETY: The first instruction is a complete 5-byte stack save. Replacing
        // the entire function requires no stolen-instruction replay. The register-
        // preserving indirect jump tail-calls the exact Win64 (RCX, EDX) signature;
        // the Rust leaf returns to the original caller using its existing stack.
        unsafe {
            session
                .detour_trampoline(FONT_SET_COLOR, 5, trampoline)
                .map_err(|error| error.to_string())?;
            if config.hud_spatial_queries {
                hud_hooks::prepare(&mut session)?;
            }
            if config.kaz_fast_scan {
                kaz_scan::prepare(&mut session)?;
            }
            if config.frame_skip {
                frame_skip::prepare(&mut session)?;
            }
        }
    }
    unsafe {
        session
            .install_permanently()
            .map_err(|error| error.to_string())?;
    }
    if config.profiling {
        profile::activate();
    }
    Ok(())
}

unsafe extern "win64" fn profiled_font_color(font: *mut u8, color: u32) {
    profile::count(4);
    // SAFETY: forwards the unchanged native entry's arguments and lifetime.
    unsafe { font_color::set_color(font, color) };
}
