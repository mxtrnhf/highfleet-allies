//! Applies per-aircraft health after the game initializes a launched aircraft.

use std::{arch::naked_asm, collections::HashMap, mem::offset_of, slice, sync::OnceLock};

#[cfg(feature = "1_151")]
use highfleet::v1_151::Body;
#[cfg(not(feature = "1_151"))]
use highfleet::v1_163::Body;
use patchy::{PatchSession, Result, ReturnType};

use crate::config::DEFAULT_PLANE_HEALTH;

const ORIGINAL_BYTES: [u8; 6] = [0x8b, 0x8e, 0x88, 0x01, 0x00, 0x00];

#[cfg(feature = "1_151")]
const HOOK_ADDRESS: usize = 0x14002f641;
#[cfg(not(feature = "1_151"))]
const HOOK_ADDRESS: usize = 0x14002ee18;

#[cfg(feature = "1_151")]
const HEALTH_OFFSET: usize = 0x2b8;
#[cfg(not(feature = "1_151"))]
const HEALTH_OFFSET: usize = 0x2c8;

const _: () = {
    assert!(offset_of!(Body, oid) == 0x190);
    assert!(offset_of!(Body, health) == HEALTH_OFFSET);
    assert!(offset_of!(Body, health_max) == HEALTH_OFFSET + 4);
};

static PLANE_HEALTH: OnceLock<HashMap<String, f32>> = OnceLock::new();

pub(crate) fn install_plane_health(configured_health: HashMap<String, f32>) {
    let mut health_overrides = HashMap::new();

    for (plane_name, health) in configured_health {
        if !is_valid_health(health) {
            log::warn!(
                "plane_health: ignoring invalid health {health} for plane '{plane_name}'; using {DEFAULT_PLANE_HEALTH}"
            );
            continue;
        }

        if health != DEFAULT_PLANE_HEALTH {
            health_overrides.insert(plane_name, health);
        }
    }

    if PLANE_HEALTH.set(health_overrides).is_err() {
        log::error!("plane_health: plane health definitions were already initialized");
    }
}

/// Prepares the aircraft-health hook when at least one custom value is configured.
pub unsafe fn patch_plane_health(session: &mut PatchSession) -> Result {
    let Some(health_overrides) = PLANE_HEALTH.get() else {
        log::error!("plane_health: plane health definitions were not initialized");
        return Ok(());
    };

    if health_overrides.is_empty() {
        log::info!("No custom aircraft health configured, skipping patch.");
        return Ok(());
    }

    let found_bytes = slice::from_raw_parts(HOOK_ADDRESS as *const u8, ORIGINAL_BYTES.len());
    if found_bytes != ORIGINAL_BYTES {
        log::error!(
            "plane_health: unexpected bytes at {HOOK_ADDRESS:#x}; expected {:02x?}, found {:02x?}",
            ORIGINAL_BYTES,
            found_bytes
        );
        return Ok(());
    }

    session.patch_call(
        HOOK_ADDRESS,
        apply_plane_health_bridge as *const (),
        ORIGINAL_BYTES.len(),
        true,
        ReturnType::None,
    )?;

    log::info!("Prepared custom aircraft-health hook at {HOOK_ADDRESS:#x}");
    Ok(())
}

/// At the hook site RSI contains the aircraft body. The overwritten instruction is
/// replayed before this bridge runs, and patchy restores its ECX result afterward.
#[unsafe(naked)]
unsafe extern "C" fn apply_plane_health_bridge() {
    naked_asm!(
        "mov rcx, rsi",
        "jmp {helper}",
        helper = sym apply_plane_health,
    );
}

unsafe extern "C" fn apply_plane_health(aircraft: *mut Body) {
    if aircraft.is_null() {
        return;
    }

    // SAFETY: The launch hook runs on the game thread with a live, aligned
    // aircraft Body. Its OID storage stays live during this callback.
    let Some(oid) = (unsafe { aircraft_oid(std::ptr::addr_of!((*aircraft).oid).cast()) }) else {
        return;
    };
    let Some(&health) = PLANE_HEALTH.get().and_then(|planes| planes.get(oid)) else {
        return;
    };

    // SAFETY: These are the native launch-health fields; the game thread owns
    // their initialization here. No reference to the whole Body is created.
    unsafe {
        std::ptr::addr_of_mut!((*aircraft).health).write(health);
        std::ptr::addr_of_mut!((*aircraft).health_max).write(health);
    }

    log::debug!("plane_health: configured plane '{oid}' with {health} health");
}

/// Borrows a native MSVC string without copying ownership or panicking on UTF-8.
/// The caller keeps the string header and its initialized storage alive and
/// unmodified for the duration of the returned borrow.
unsafe fn aircraft_oid<'a>(string: *const u8) -> Option<&'a str> {
    const INLINE_CAPACITY: usize = 15;
    const MAX_OID_LENGTH: usize = 0x1000;

    // SAFETY: The caller supplies the aligned, live EscadraString field. Its
    // MSVC layout is the 16-byte inline/pointer union followed by length/capacity.
    let length = unsafe { string.add(0x10).cast::<usize>().read() };
    let capacity = unsafe { string.add(0x18).cast::<usize>().read() };
    if length > capacity || length > MAX_OID_LENGTH {
        return None;
    }
    let data = if capacity <= INLINE_CAPACITY {
        string
    } else {
        unsafe { string.cast::<*const u8>().read() }
    };
    if data.is_null() {
        return None;
    }

    std::str::from_utf8(unsafe { slice::from_raw_parts(data, length) }).ok()
}

fn is_valid_health(health: f32) -> bool {
    health.is_finite() && health > 0.0
}
